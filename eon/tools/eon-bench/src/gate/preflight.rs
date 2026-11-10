use super::{Options, Result, error, fixed_conditions, object, read};
use crate::{
    environment,
    json::Value,
    result::{Grid, RunResult},
    suite, terminals,
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

/// Probe a sibling, retaining create_new on the final write to cover races.
pub(super) fn record_destination(path: &Path) -> Result {
    match fs::symlink_metadata(path) {
        Ok(_) => return Err("baseline exists; retain it or choose a distinct file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(e)),
    }
    if path.file_name().is_none() {
        return Err("baseline needs a filename".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let probe = parent.join(format!(
        ".eon-bench-probe-{}-{}",
        std::process::id(),
        environment::monotonic_ns()
    ));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| format!("baseline directory {}: {e}", parent.display()))?;
    drop(file);
    fs::remove_file(probe).map_err(error)
}

pub(super) fn conditions(
    options: &Options,
    cpus: &str,
    max_load: &str,
    exe: &Path,
) -> Result<Value> {
    let eon = crate::launcher::candidate(&options.eon)?;
    terminals::check_candidate_fontconfig(&eon, &terminals::tool("product_fontconfig")?)?;
    let adapter = options.out.join("preflight-adapter");
    let log = fs::File::create(options.out.join("preflight-adapter.log")).map_err(error)?;
    let status = environment::die_with_parent(
        Command::new(exe)
            .args(["headless", "--eon"])
            .arg(eon)
            .arg("--out")
            .arg(&adapter)
            .args(["--renderer", "lavapipe"])
            .env_remove("EON_BENCH_TRACE")
            .env_remove("EON_BENCH_TRACE_BLOCKING"),
    )
    .stdin(Stdio::null())
    .stdout(log.try_clone().map_err(error)?)
    .stderr(log)
    .status()
    .map_err(error)?;
    if !status.success() {
        return Err(format!(
            "adapter preflight: {status}; see preflight-adapter.log"
        ));
    }
    let adapter = read(&adapter.join("result.json"))?;
    let max_load = max_load.parse::<f64>().map_err(error)?;
    if !max_load.is_finite() || max_load <= 0.0 {
        return Err("invalid max_load".into());
    }
    let settings = suite::Settings {
        out: options.out.clone(),
        samples: super::SAMPLES,
        warmups: super::WARMUPS,
        max_load,
        load_wait: Duration::ZERO,
        cpus: Some(environment::parse_cpus(cpus)?),
        quick: false,
    };
    let font = terminals::tool("font")?;
    let kitten = terminals::tool("kitten")?;
    let mut suites = Vec::new();
    for name in ["orbit", "terminals"] {
        let tools = if name == "terminals" {
            terminals::fixed_tools(&font, &kitten, 10)?
        } else {
            Vec::new()
        };
        let mut raw = RunResult {
            suite: name.into(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            quick: false,
            corpus_version: crate::corpus::CORPUS_VERSION as u64,
            warmups: super::WARMUPS,
            samples: super::SAMPLES,
            started_at: String::new(),
            finished_at: String::new(),
            cells: Vec::new(),
            environment: suite::environment(
                &settings,
                environment::kernel(),
                environment::load()?,
                Grid { cols: 80, rows: 24 },
                tools,
                environment::build()?,
            )?,
        }
        .to_json();
        if name == "terminals"
            && let Value::Object(fields) = &mut raw
        {
            let extra = object([
                ("requested_renderer", "lavapipe".into()),
                ("font_family", "DejaVu Sans Mono".into()),
                (
                    "fontconfig",
                    terminals::tool("fontconfig")?.display().to_string().into(),
                ),
                (
                    "product_fontconfig_mount",
                    terminals::tool("product_fontconfig")?
                        .display()
                        .to_string()
                        .into(),
                ),
                ("adapter_preflight", adapter.clone()),
            ]);
            if let Value::Object(extra) = extra {
                fields.extend(extra);
            }
        }
        suites.push((name.into(), fixed_conditions(name, &raw)?));
    }
    let conditions = Value::Object(suites);
    fs::write(
        options.out.join("preflight-conditions.json"),
        conditions.to_pretty(),
    )
    .map_err(error)?;
    Ok(conditions)
}
