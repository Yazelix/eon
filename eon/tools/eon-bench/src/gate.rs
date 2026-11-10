//! Same-machine completion timing gate. Raw data stays with the existing suites.
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{environment, json::Value, result::RunResult, sha256::Sha256, stats::Summary};

#[cfg(test)]
mod checks;
mod power;
mod preflight;

type Result<T = ()> = std::result::Result<T, String>;
const SAMPLES: u64 = 10;
const WARMUPS: u64 = 2;
const USAGE: &str = "usage: eon-bench gate --baseline FILE --eon PATH --source-tree HASH [--record-baseline] [--out NEW_DIR] [--orbit PATH] [--cpus LIST] [--max-load LOAD]";

pub struct Options {
    baseline: PathBuf,
    record: bool,
    out: PathBuf,
    eon: PathBuf,
    tree: String,
    orbit: Option<PathBuf>,
    cpus: Option<String>,
    max_load: Option<String>,
}

pub fn options(args: &[String]) -> Result<Options> {
    let mut flags = BTreeMap::new();
    let mut record = false;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                println!(
                    "{USAGE}\n\nRuns all six corpora through Orbit detached/attached, then Eon's six\nrender-enabled completion workloads on private Sway (lavapipe, 80x24).\n10 samples after 2 warmups per cell, 10 kitten repetitions. Requires release bench and candidate\nfrom the declared tree. No desktop/profile access or build during timing.\n\nComparison always makes two complete runs. Fails if the SAME cell's p50\nAND nearest-rank p95 both exceed baseline by MORE than 25% in BOTH runs.\nIncomplete or mismatched evidence fails. This is not pixel/frame latency.\n\n--record-baseline records one complete run; FILE must not exist. Recording\nrequires --cpus; comparisons inherit its CPU set and load ceiling unless\nexplicitly supplied (they must match). Other conditions must also match.\nAC state is recorded before/after every run; missing, changed or baseline-\nmismatched Mains power invalidates timing.\nWithout --out, creates a fresh directory under ~/.cache/eon-bench/.\nExit 0: pass/recorded; 1: regression or invalid run; 2: usage.\nLinux only until a display suite and baseline exist on another platform."
                );
                return Err(String::new());
            }
            "--record-baseline" if !record => record = true,
            "--baseline" | "--eon" | "--source-tree" | "--out" | "--orbit" | "--cpus"
            | "--max-load" => {
                let value = args.next().ok_or(format!("{flag} needs a value"))?;
                if flags.insert(flag.as_str(), value.clone()).is_some() {
                    return Err(format!("{flag} is repeated"));
                }
            }
            _ => return Err(format!("unknown/repeated argument {flag}\n{USAGE}")),
        }
    }
    let required = |key| {
        flags
            .get(key)
            .cloned()
            .ok_or(format!("{key} is required\n{USAGE}"))
    };
    let tree = required("--source-tree")?;
    if tree.len() != 40 || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("--source-tree needs a 40-digit git tree hash".into());
    }
    if record && !flags.contains_key("--cpus") {
        return Err("recording requires --cpus LIST".into());
    }
    let default_out = || {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .ok_or("set --out or HOME/XDG_CACHE_HOME")?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(error)?
            .as_nanos();
        Ok::<_, String>(cache.join(format!("eon-bench/gate-{stamp}-{}", std::process::id())))
    };
    Ok(Options {
        baseline: required("--baseline")?.into(),
        eon: required("--eon")?.into(),
        tree: tree.to_lowercase(),
        record,
        out: flags
            .get("--out")
            .map(PathBuf::from)
            .map_or_else(default_out, Ok)?,
        orbit: flags.get("--orbit").map(PathBuf::from),
        cpus: flags.get("--cpus").cloned(),
        max_load: flags.get("--max-load").cloned(),
    })
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k).ok_or(format!("missing {k}"))
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    field(v, k)?.as_str().ok_or(format!("invalid {k}"))
}
fn read(path: &Path) -> Result<Value> {
    Value::parse(&fs::read_to_string(path).map_err(error)?)
}
fn machine() -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(&fs::read("/etc/machine-id").map_err(|e| format!("local machine-id: {e}"))?);
    Ok(hash.hex())
}

#[derive(Debug, PartialEq)]
struct Timing {
    p50: f64,
    p95: f64,
}
fn timing(v: &Value) -> Result<Timing> {
    let number = |key| {
        field(v, key)?
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .ok_or(format!("invalid {key}"))
    };
    let t = Timing {
        p50: number("p50")?,
        p95: number("p95")?,
    };
    if field(v, "n")?.as_u64() != Some(SAMPLES) || t.p95 < t.p50 {
        return Err("invalid sample count/percentiles".into());
    }
    Ok(t)
}
fn regressed(base: &Timing, runs: [&Timing; 2]) -> bool {
    runs.iter()
        .all(|t| t.p50 > base.p50 * 1.25 && t.p95 > base.p95 * 1.25)
}
fn cells(snapshot: &Value) -> Result<BTreeMap<String, Timing>> {
    let mut result = BTreeMap::new();
    for cell in field(snapshot, "cells")?
        .as_array()
        .ok_or("cells is not an array")?
    {
        let key = text(cell, "key")?.to_owned();
        if result.insert(key.clone(), timing(cell)?).is_some() {
            return Err(format!("duplicate cell {key}"));
        }
    }
    if result.len() != 18 {
        return Err("expected 18 complete gate cells".into());
    }
    Ok(result)
}

fn adapter_conditions(adapter: &Value) -> Result<Value> {
    if text(adapter, "result")? != "pass" {
        return Err("adapter preflight did not pass".into());
    }
    Ok(Value::Object(
        [
            "compositor",
            "compositor_version",
            "compositor_renderer",
            "vulkan_icd",
        ]
        .into_iter()
        .map(|key| {
            let identity = text(adapter, key)?;
            if identity.is_empty() {
                return Err(format!("unavailable adapter {key}"));
            }
            Ok((key.into(), identity.into()))
        })
        .collect::<Result<Vec<_>>>()?,
    ))
}

fn fixed_conditions(suite: &str, raw: &Value) -> Result<Value> {
    let env = field(raw, "environment")?;
    let mut fixed = vec![
        (
            "corpus_version".into(),
            field(raw, "corpus_version")?.clone(),
        ),
        (
            "corpora".into(),
            Value::Array(
                crate::corpus::CORPORA
                    .iter()
                    .map(|c| {
                        object([
                            ("id", c.id.into()),
                            ("sha256", c.sha256.into()),
                            ("bytes", c.length.into()),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    for key in [
        "os",
        "cpu",
        "cpus_online",
        "pinned_cpus",
        "cpus_allowed",
        "governor",
        "kernel",
        "max_load",
        "grid",
    ] {
        let value = field(env, key)?;
        if matches!(value, Value::Null) {
            return Err(format!("{suite}: unavailable {key}"));
        }
        fixed.push((key.into(), value.clone()));
    }
    if suite == "terminals" {
        for key in [
            "requested_renderer",
            "font_family",
            "fontconfig",
            "product_fontconfig_mount",
        ] {
            fixed.push((key.into(), field(raw, key)?.clone()));
        }
        let tools = field(env, "tools")?;
        for key in [
            "font-file",
            "font-px",
            "scrollback",
            "kitten",
            "kitten-repetitions",
        ] {
            fixed.push((key.into(), field(tools, key)?.clone()));
        }
        fixed.push((
            "adapter".into(),
            adapter_conditions(field(raw, "adapter_preflight")?)?,
        ));
    }
    Ok(Value::Object(fixed))
}

fn check_conditions(current: &Value, baseline: &Value) -> Result {
    if current != baseline {
        return Err("baseline conditions differ; see preflight/raw environment blocks (do not replace baseline with candidate timings)".into());
    }
    Ok(())
}

/// Reject a partial suite and recompute statistics from the samples, never cached stats.
fn snapshot(results: &[Value], machine_id: &str) -> Result<Value> {
    let mut conditions = Vec::new();
    let mut measured_cells = Vec::new();
    if results.len() != 2 {
        return Err("expected both complete suites".into());
    }
    for (suite, raw) in ["orbit", "terminals"].into_iter().zip(results) {
        let run = RunResult::from_json(raw)?;
        if run.suite != suite
            || run.quick
            || run.samples != SAMPLES
            || run.warmups != WARMUPS
            || run.corpus_version != crate::corpus::CORPUS_VERSION as u64
            || run.environment.build.profile != "release"
            || run.environment.grid.rows != 24
            || run.environment.grid.cols != 80
        {
            return Err(format!("{suite}: incomplete or mismatched settings"));
        }
        if suite == "terminals" && text(raw, "result")? != "pass" {
            return Err("terminals did not pass".into());
        }
        let mut expected = if suite == "orbit" {
            crate::corpus::CORPORA
                .iter()
                .flat_map(|c| {
                    [
                        format!("{}/orbit-detached/yazelix-orbit", c.id),
                        format!("{}/orbit-attached/yazelix-orbit", c.id),
                    ]
                })
                .collect::<Vec<_>>()
        } else {
            std::iter::once("cat64".to_owned())
                .chain(
                    crate::kitten::BENCHMARKS
                        .iter()
                        .map(|n| format!("kitten-{n}")),
                )
                .map(|w| format!("{w}/render-enabled-completion/eon"))
                .collect()
        };
        for cell in &run.cells {
            let key = format!("{}/{}/{}", cell.workload, cell.layer, cell.subject);
            let at = expected
                .iter()
                .position(|k| k == &key)
                .ok_or(format!("unexpected/duplicate {suite}/{key}"))?;
            expected.remove(at);
            if cell.samples.len() != SAMPLES as usize
                || cell.samples.iter().any(|s| {
                    !s.seconds.is_finite()
                        || s.seconds <= 0.0
                        || !s.load.is_finite()
                        || s.load > run.environment.max_load
                })
            {
                return Err(format!("incomplete/invalid {suite}/{key}"));
            }
            if let Some(corpus) = crate::corpus::find(&cell.workload)
                && (cell.corpus_sha256 != corpus.sha256 || cell.input_bytes != corpus.length)
            {
                return Err(format!("changed corpus {key}"));
            }
            let times = cell.samples.iter().map(|s| s.seconds).collect::<Vec<_>>();
            let stats = Summary::of(&times).ok_or("empty cell")?;
            measured_cells.push(object([
                ("key", format!("{suite}/{key}").into()),
                ("n", SAMPLES.into()),
                ("p50", stats.median.into()),
                ("p95", stats.p95.into()),
            ]));
        }
        if !expected.is_empty() {
            return Err(format!("{suite}: missing {expected:?}"));
        }
        conditions.push((suite.to_owned(), fixed_conditions(suite, raw)?));
    }
    let artifacts = results
        .iter()
        .map(|r| {
            let env = field(r, "environment")?;
            Ok(object([
                ("suite", field(r, "suite")?.clone()),
                ("build", field(env, "build")?.clone()),
                ("tools", field(env, "tools")?.clone()),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let result = object([
        ("schema", 1u64.into()),
        ("tool", "eon-bench-gate".into()),
        ("machine", machine_id.into()),
        ("conditions", Value::Object(conditions)),
        ("cells", Value::Array(measured_cells)),
        ("artifacts", Value::Array(artifacts)),
    ]);
    cells(&result)?;
    Ok(result)
}

pub fn run(options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("timing gate currently requires Linux and private Sway".into());
    }
    let baseline = if options.record {
        preflight::record_destination(&options.baseline)?;
        None
    } else {
        let b = read(&options.baseline)?;
        if field(&b, "schema")?.as_u64() != Some(1) || text(&b, "tool")? != "eon-bench-gate" {
            return Err("unsupported gate baseline".into());
        }
        if text(&b, "machine")? != machine()? {
            return Err(
                "baseline belongs to another machine; record from the parent revision here first"
                    .into(),
            );
        }
        cells(&b)?;
        power::recorded(&b)?;
        Some(b)
    };
    let inherited = |key: &str| -> Result<String> {
        let b = baseline
            .as_ref()
            .ok_or(format!("recording requires {key}"))?;
        let orbit = field(field(b, "conditions")?, "orbit")?;
        if key == "cpus" {
            let values = field(orbit, "pinned_cpus")?
                .as_array()
                .ok_or("baseline has no pinned CPUs")?;
            let cpus = values
                .iter()
                .map(|v| {
                    v.as_u64()
                        .map(|n| n.to_string())
                        .ok_or("invalid CPU".to_owned())
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(cpus.join(","))
        } else {
            Ok(field(orbit, "max_load")?
                .as_f64()
                .ok_or("invalid max_load")?
                .to_string())
        }
    };
    let cpus = options.cpus.clone().map_or_else(|| inherited("cpus"), Ok)?;
    let max_load = options.max_load.clone().map_or_else(
        || {
            if options.record {
                Ok((environment::cpus_online() as f64 / 4.0)
                    .max(1.0)
                    .to_string())
            } else {
                inherited("max_load")
            }
        },
        Ok,
    )?;
    let parsed_cpus = environment::parse_cpus(&cpus)?;
    if parsed_cpus.is_empty() {
        return Err("no CPUs".into());
    }
    environment::pin(&parsed_cpus)?;
    if environment::build()?.profile != "release" {
        return Err("timing requires a release eon-bench".into());
    }
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    fs::create_dir(&options.out).map_err(|e| format!("{}: {e}", options.out.display()))?;
    let exe = std::env::current_exe().map_err(error)?;
    let current_conditions = preflight::conditions(options, &cpus, &max_load, &exe)?;
    if let Some(b) = &baseline {
        check_conditions(&current_conditions, field(b, "conditions")?)?;
    }
    let machine_id = machine()?;
    let mut runs = Vec::new();

    for round in 1..=if options.record { 1 } else { 2 } {
        let (results, ac_power) = power::measure(
            Path::new("/sys/class/power_supply"),
            &options.out.join(format!("run-{round}-power.json")),
            baseline.as_ref().map(power::recorded).transpose()?,
            || {
                let mut results = Vec::new();
                for suite in ["orbit", "terminals"] {
                    let out = options.out.join(format!("run-{round}-{suite}"));
                    let mut command = Command::new(&exe);
                    command
                        .env_remove("EON_BENCH_TRACE")
                        .env_remove("EON_BENCH_TRACE_BLOCKING");
                    command.arg(suite).arg("--out").arg(&out).args([
                        "--samples",
                        "10",
                        "--warmups",
                        "2",
                        "--cpus",
                        &cpus,
                        "--max-load",
                        &max_load,
                    ]);
                    if suite == "orbit" {
                        if let Some(orbit) = &options.orbit {
                            command.arg("--orbit").arg(orbit);
                        }
                        for corpus in crate::corpus::CORPORA {
                            command.args(["--only", corpus.id]);
                        }
                    } else {
                        command.arg("--eon").arg(&options.eon).args([
                            "--source-tree",
                            &options.tree,
                            "--subject",
                            "eon",
                            "--renderer",
                            "lavapipe",
                            "--kitten-repetitions",
                            "10",
                        ]);
                    }
                    let log =
                        fs::File::create(options.out.join(format!("run-{round}-{suite}.log")))
                            .map_err(error)?;
                    let log2 = log.try_clone().map_err(error)?;
                    eprintln!(
                        "eon-bench gate: run {round} {suite}; log {}",
                        options.out.display()
                    );
                    let status = environment::die_with_parent(&mut command)
                        .stdin(Stdio::null())
                        .stdout(log)
                        .stderr(log2)
                        .status()
                        .map_err(error)?;
                    if !status.success() {
                        return Err(format!(
                            "run {round} {suite}: {status}; see {}",
                            options.out.display()
                        ));
                    }
                    results.push(read(&out.join("result.json"))?);
                }
                Ok(results)
            },
        )?;
        let mut current = snapshot(&results, &machine_id)?;
        if let Value::Object(fields) = &mut current {
            fields.push(("ac_power".into(), ac_power));
        }
        check_conditions(field(&current, "conditions")?, &current_conditions)?;
        if let Some(b) = &baseline {
            check_conditions(field(&current, "conditions")?, field(b, "conditions")?)?;
        }
        fs::write(
            options.out.join(format!("run-{round}.json")),
            current.to_pretty(),
        )
        .map_err(error)?;
        runs.push(current);
    }
    if options.record {
        let mut b = runs.remove(0);
        if let Value::Object(fields) = &mut b {
            fields.extend([
                ("source_tree".into(), options.tree.as_str().into()),
                ("evidence".into(), options.out.display().to_string().into()),
            ]);
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&options.baseline)
            .map_err(error)?;
        file.write_all(b.to_pretty().as_bytes()).map_err(error)?;
        println!("baseline: {}", options.baseline.display());
        return Ok(());
    }
    let b = cells(baseline.as_ref().unwrap())?;
    let first = cells(&runs[0])?;
    let second = cells(&runs[1])?;
    let mut regressions = Vec::new();
    for (key, base) in &b {
        let a = first.get(key).ok_or(format!("run 1 missing {key}"))?;
        let c = second.get(key).ok_or(format!("run 2 missing {key}"))?;
        if regressed(base, [a, c]) {
            regressions.push(Value::from(key.as_str()));
        }
    }
    let pass = regressions.is_empty();
    let result = object([
        ("schema", 1u64.into()),
        ("tool", "eon-bench-gate".into()),
        ("baseline", options.baseline.display().to_string().into()),
        ("source_tree", options.tree.as_str().into()),
        ("result", if pass { "pass" } else { "fail" }.into()),
        ("regressions", Value::Array(regressions)),
        (
            "artifacts",
            Value::Array(
                runs.iter()
                    .map(|r| field(r, "artifacts").cloned())
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        ("runs", Value::Array(runs)),
    ]);
    fs::write(options.out.join("result.json"), result.to_pretty()).map_err(error)?;
    println!(
        "{}: {}",
        if pass { "PASS" } else { "FAIL" },
        options.out.display()
    );
    if pass {
        Ok(())
    } else {
        Err("confirmed timing regression; see result.json".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmation_requires_both_statistics_in_both_runs_strictly_above_25_percent() {
        let base = Timing { p50: 1.0, p95: 2.0 };
        let slow = Timing {
            p50: 1.26,
            p95: 2.52,
        };
        let boundary = Timing {
            p50: 1.25,
            p95: 2.5,
        };
        let tail_only = Timing { p50: 1.0, p95: 3.0 };
        assert!(regressed(&base, [&slow, &slow]));
        for other in [&base, &boundary, &tail_only] {
            assert!(!regressed(&base, [&slow, other]));
            assert!(!regressed(&base, [other, &slow]));
        }
    }
    #[test]
    fn baseline_rejects_missing_duplicate_or_invalid_statistics() {
        let valid = object([
            ("n", 10u64.into()),
            ("p50", 1.0.into()),
            ("p95", 2.0.into()),
        ]);
        assert!(timing(&valid).is_ok());
        for bad in [
            "{}",
            "{\"n\":9,\"p50\":1,\"p95\":2}",
            "{\"n\":10,\"p50\":0,\"p95\":2}",
            "{\"n\":10,\"p50\":2,\"p95\":1}",
            "{\"n\":10,\"p50\":null,\"p95\":2}",
        ] {
            assert!(timing(&Value::parse(bad).unwrap()).is_err());
        }
        assert!(cells(&object([("cells", Value::Array(vec![]))])).is_err());
        let cell = object([
            ("key", "same".into()),
            ("n", 10u64.into()),
            ("p50", 1.0.into()),
            ("p95", 2.0.into()),
        ]);
        assert!(
            cells(&object([("cells", Value::Array(vec![cell; 18]))]))
                .unwrap_err()
                .contains("duplicate")
        );
    }
}
