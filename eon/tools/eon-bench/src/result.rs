//! `result.json`, schema 1: one file per suite run with its settings, the
//! environment it ran in, and one cell per workload, layer and subject holding
//! every sample. The samples own the data; each cell's `stats` and `counters`
//! summaries are written for readers and recomputed, not read back.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{json::Value, stats::Summary};

pub const SCHEMA: u64 = 1;
const TOOL: &str = "eon-bench";

#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub suite: String,
    pub tool_version: String,
    pub quick: bool,
    pub corpus_version: u64,
    pub warmups: u64,
    pub samples: u64,
    /// RFC 3339 UTC times.
    pub started_at: String,
    pub finished_at: String,
    pub environment: Environment,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    /// The operating system and its version.
    pub os: Option<String>,
    pub cpu: Option<String>,
    pub cpus_online: u64,
    /// The CPUs `--cpus` pinned the run to; `None` when it was not pinned.
    pub pinned_cpus: Option<Vec<u64>>,
    /// The CPUs the run could use, where the platform reports them.
    pub cpus_allowed: Option<Vec<u64>>,
    /// The distinct frequency governors of the allowed CPUs, comma-separated.
    pub governor: Option<String>,
    pub kernel: String,
    /// The 1-minute load average at the start and end of the run, and the
    /// limit above which a sample waits.
    pub load_start: f64,
    pub load_end: f64,
    pub max_load: f64,
    pub grid: Grid,
    /// Each tool the run used, with its version.
    pub tools: Vec<(String, String)>,
    pub build: Build,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub cols: u64,
    pub rows: u64,
}

/// The eon-bench binary that ran and the Eon source it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub path: String,
    pub sha256: String,
    /// `release` or `debug`.
    pub profile: String,
    pub source: Option<Source>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub commit: String,
    /// Whether tracked files differed from the commit.
    pub dirty: bool,
    /// `build` when the Nix build recorded the commit, `checkout-at-run` when
    /// it was read from the Eon checkout the binary was built in as the run
    /// started, which may have moved since the build.
    pub from: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub workload: String,
    pub corpus_sha256: String,
    /// The bytes the workload writes, which throughput is counted in.
    pub input_bytes: u64,
    pub layer: String,
    pub subject: String,
    pub subject_version: String,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub seconds: f64,
    /// The 1-minute load average when the sample started.
    pub load: f64,
    pub counters: Vec<(String, u64)>,
}

impl RunResult {
    pub fn to_json(&self) -> Value {
        let environment = &self.environment;
        let build = &environment.build;
        let cpus = |cpus: &Option<Vec<u64>>| {
            cpus.as_ref().map_or(Value::Null, |cpus| {
                Value::Array(cpus.iter().map(|&cpu| cpu.into()).collect())
            })
        };
        object([
            ("schema", SCHEMA.into()),
            ("tool", TOOL.into()),
            ("tool_version", self.tool_version.as_str().into()),
            ("suite", self.suite.as_str().into()),
            ("quick", self.quick.into()),
            ("corpus_version", self.corpus_version.into()),
            ("warmups", self.warmups.into()),
            ("samples", self.samples.into()),
            ("started_at", self.started_at.as_str().into()),
            ("finished_at", self.finished_at.as_str().into()),
            (
                "environment",
                object([
                    ("os", environment.os.as_deref().into()),
                    ("cpu", environment.cpu.as_deref().into()),
                    ("cpus_online", environment.cpus_online.into()),
                    ("pinned_cpus", cpus(&environment.pinned_cpus)),
                    ("cpus_allowed", cpus(&environment.cpus_allowed)),
                    ("governor", environment.governor.as_deref().into()),
                    ("kernel", environment.kernel.as_str().into()),
                    ("load_start", environment.load_start.into()),
                    ("load_end", environment.load_end.into()),
                    ("max_load", environment.max_load.into()),
                    (
                        "grid",
                        object([
                            ("cols", environment.grid.cols.into()),
                            ("rows", environment.grid.rows.into()),
                        ]),
                    ),
                    (
                        "tools",
                        Value::Object(
                            environment
                                .tools
                                .iter()
                                .map(|(tool, version)| (tool.clone(), version.as_str().into()))
                                .collect(),
                        ),
                    ),
                    (
                        "build",
                        object([
                            ("path", build.path.as_str().into()),
                            ("sha256", build.sha256.as_str().into()),
                            ("profile", build.profile.as_str().into()),
                            (
                                "source",
                                build.source.as_ref().map_or(Value::Null, |source| {
                                    object([
                                        ("commit", source.commit.as_str().into()),
                                        ("dirty", source.dirty.into()),
                                        ("from", source.from.as_str().into()),
                                    ])
                                }),
                            ),
                        ]),
                    ),
                ]),
            ),
            (
                "cells",
                Value::Array(self.cells.iter().map(Cell::to_json).collect()),
            ),
        ])
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let schema = u64_field(value, "schema")?;
        if schema != SCHEMA {
            return Err(format!(
                "result schema {schema}; this eon-bench reads {SCHEMA}"
            ));
        }
        if str_field(value, "tool")? != TOOL {
            return Err(format!("result is not from {TOOL}"));
        }
        let environment = field(value, "environment")?;
        let grid = field(environment, "grid")?;
        let build = field(environment, "build")?;
        let source = field(build, "source")?;
        Ok(Self {
            suite: string_field(value, "suite")?,
            tool_version: string_field(value, "tool_version")?,
            quick: bool_field(value, "quick")?,
            corpus_version: u64_field(value, "corpus_version")?,
            warmups: u64_field(value, "warmups")?,
            samples: u64_field(value, "samples")?,
            started_at: string_field(value, "started_at")?,
            finished_at: string_field(value, "finished_at")?,
            environment: Environment {
                os: optional(environment, "os", |v| v.as_str().map(str::to_owned))?,
                cpu: optional(environment, "cpu", |v| v.as_str().map(str::to_owned))?,
                cpus_online: u64_field(environment, "cpus_online")?,
                pinned_cpus: optional(environment, "pinned_cpus", u64_list)?,
                cpus_allowed: optional(environment, "cpus_allowed", u64_list)?,
                governor: optional(environment, "governor", |v| v.as_str().map(str::to_owned))?,
                kernel: string_field(environment, "kernel")?,
                load_start: f64_field(environment, "load_start")?,
                load_end: f64_field(environment, "load_end")?,
                max_load: f64_field(environment, "max_load")?,
                grid: Grid {
                    cols: u64_field(grid, "cols")?,
                    rows: u64_field(grid, "rows")?,
                },
                tools: field(environment, "tools")?
                    .as_object()
                    .ok_or("environment.tools is not an object")?
                    .iter()
                    .map(|(tool, version)| {
                        version
                            .as_str()
                            .map(|version| (tool.clone(), version.to_owned()))
                            .ok_or(format!("tools.{tool} is not a string"))
                    })
                    .collect::<Result<_, _>>()?,
                build: Build {
                    path: string_field(build, "path")?,
                    sha256: string_field(build, "sha256")?,
                    profile: string_field(build, "profile")?,
                    source: match source {
                        Value::Null => None,
                        source => Some(Source {
                            commit: string_field(source, "commit")?,
                            dirty: bool_field(source, "dirty")?,
                            from: string_field(source, "from")?,
                        }),
                    },
                },
            },
            cells: array_field(value, "cells")?
                .iter()
                .map(Cell::from_json)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl Cell {
    /// The cell with its summaries: `stats.seconds` over the samples, and
    /// `stats.mb_per_s` in MB (10^6 bytes) of input per second, whose `p95` is
    /// the throughput at the p95 time, the slow tail; `counters` summarizes
    /// each counter. Each summary carries its sample count `n`; below 20
    /// samples its p95 is the slowest sample.
    pub(crate) fn to_json(&self) -> Value {
        let seconds: Vec<f64> = self.samples.iter().map(|sample| sample.seconds).collect();
        let stats = Summary::of(&seconds).map_or(Value::Null, |time| {
            let rate = |seconds: f64| Value::from(self.input_bytes as f64 / 1e6 / seconds);
            object([
                ("seconds", summary(time)),
                (
                    "mb_per_s",
                    object([
                        ("n", (time.n as u64).into()),
                        ("median", rate(time.median)),
                        ("p95", rate(time.p95)),
                        ("min", rate(time.max)),
                        ("max", rate(time.min)),
                    ]),
                ),
            ])
        });
        let mut names: Vec<&str> = Vec::new();
        for sample in &self.samples {
            for (name, _) in &sample.counters {
                if !names.contains(&name.as_str()) {
                    names.push(name);
                }
            }
        }
        let counters = names
            .into_iter()
            .map(|name| {
                let values: Vec<f64> = self
                    .samples
                    .iter()
                    .flat_map(|sample| &sample.counters)
                    .filter(|(counter, _)| counter == name)
                    .map(|&(_, value)| value as f64)
                    .collect();
                (
                    name.to_owned(),
                    Summary::of(&values).map_or(Value::Null, summary),
                )
            })
            .collect();
        object([
            ("workload", self.workload.as_str().into()),
            ("corpus_sha256", self.corpus_sha256.as_str().into()),
            ("input_bytes", self.input_bytes.into()),
            ("layer", self.layer.as_str().into()),
            ("subject", self.subject.as_str().into()),
            ("subject_version", self.subject_version.as_str().into()),
            (
                "samples",
                Value::Array(
                    self.samples
                        .iter()
                        .map(|sample| {
                            object([
                                ("seconds", sample.seconds.into()),
                                ("load", sample.load.into()),
                                (
                                    "counters",
                                    Value::Object(
                                        sample
                                            .counters
                                            .iter()
                                            .map(|(name, value)| (name.clone(), (*value).into()))
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("stats", stats),
            ("counters", Value::Object(counters)),
        ])
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        Ok(Self {
            workload: string_field(value, "workload")?,
            corpus_sha256: string_field(value, "corpus_sha256")?,
            input_bytes: u64_field(value, "input_bytes")?,
            layer: string_field(value, "layer")?,
            subject: string_field(value, "subject")?,
            subject_version: string_field(value, "subject_version")?,
            samples: array_field(value, "samples")?
                .iter()
                .map(|sample| {
                    Ok(Sample {
                        seconds: f64_field(sample, "seconds")?,
                        load: f64_field(sample, "load")?,
                        counters: field(sample, "counters")?
                            .as_object()
                            .ok_or("sample counters are not an object")?
                            .iter()
                            .map(|(name, value)| {
                                value
                                    .as_u64()
                                    .map(|value| (name.clone(), value))
                                    .ok_or(format!("counter {name} is not a whole number"))
                            })
                            .collect::<Result<_, String>>()?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn summary(summary: Summary) -> Value {
    object([
        ("n", (summary.n as u64).into()),
        ("median", summary.median.into()),
        ("p95", summary.p95.into()),
        ("min", summary.min.into()),
        ("max", summary.max.into()),
    ])
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value.get(key).ok_or(format!("result has no {key}"))
}

fn typed<'a, T>(
    value: &'a Value,
    key: &str,
    kind: &str,
    read: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<T, String> {
    read(field(value, key)?).ok_or(format!("result {key} is not {kind}"))
}

fn string_field(value: &Value, key: &str) -> Result<String, String> {
    typed(value, key, "a string", |v| v.as_str().map(str::to_owned))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    typed(value, key, "a string", Value::as_str)
}

fn u64_field(value: &Value, key: &str) -> Result<u64, String> {
    typed(value, key, "a whole number", Value::as_u64)
}

fn f64_field(value: &Value, key: &str) -> Result<f64, String> {
    typed(value, key, "a number", Value::as_f64)
}

fn bool_field(value: &Value, key: &str) -> Result<bool, String> {
    typed(value, key, "a boolean", Value::as_bool)
}

fn array_field<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    typed(value, key, "an array", Value::as_array)
}

/// A field that may be null.
fn optional<T>(
    value: &Value,
    key: &str,
    read: impl FnOnce(&Value) -> Option<T>,
) -> Result<Option<T>, String> {
    match field(value, key)? {
        Value::Null => Ok(None),
        present => read(present)
            .map(Some)
            .ok_or(format!("result {key} has the wrong type")),
    }
}

fn u64_list(value: &Value) -> Option<Vec<u64>> {
    value.as_array()?.iter().map(Value::as_u64).collect()
}

/// `time` as RFC 3339 UTC to the second, such as `2026-11-07T18:40:00Z`.
pub fn utc_timestamp(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{Build, Cell, Environment, Grid, RunResult, Sample, Source, utc_timestamp};
    use crate::json::Value;

    fn result() -> RunResult {
        let cell = |workload: &str, samples: Vec<Sample>| Cell {
            workload: workload.into(),
            corpus_sha256: "6ebf5259297fd841eaf4471f48ca6885e33ad705acb20f005728a0af7c9bcae8"
                .into(),
            input_bytes: 16_777_289,
            layer: "pty".into(),
            subject: "kernel-pty".into(),
            subject_version: "Linux 7.2.6 x86_64".into(),
            samples,
        };
        let sample = |seconds: f64, reads: u64| Sample {
            seconds,
            load: 1.25,
            counters: vec![
                ("pty_bytes".into(), 17_056_000),
                ("pty_reads".into(), reads),
            ],
        };
        RunResult {
            suite: "floor".into(),
            tool_version: "0.1.0".into(),
            quick: false,
            corpus_version: 1,
            warmups: 2,
            samples: 3,
            started_at: "2026-11-07T18:40:00Z".into(),
            finished_at: "2026-11-07T18:41:07Z".into(),
            environment: Environment {
                os: Some("Pop!_OS 24.04 LTS".into()),
                cpu: Some("AMD Ryzen 7 \"test\"".into()),
                cpus_online: 16,
                pinned_cpus: Some(vec![2, 3]),
                cpus_allowed: Some(vec![2, 3]),
                governor: Some("performance".into()),
                kernel: "Linux 7.2.6 x86_64".into(),
                load_start: 0.5,
                load_end: 1.0 / 3.0,
                max_load: 4.0,
                grid: Grid { cols: 80, rows: 24 },
                tools: vec![("eon-bench".into(), "0.1.0".into())],
                build: Build {
                    path:
                        "/nix/store/nzh5j6yaws9ypgbyqspk6cf43i1pad1w-eon-bench-0.1.0/bin/eon-bench"
                            .into(),
                    sha256: "5d93c4f2579ed7332d9e9d452e816bdbb5fe7f534cd8b73e60027c6196c7010d"
                        .into(),
                    profile: "release".into(),
                    source: Some(Source {
                        commit: "5eb38bfa31306f2b5102f754a762424c34467216".into(),
                        dirty: true,
                        from: "build".into(),
                    }),
                },
            },
            cells: vec![
                cell(
                    "ascii",
                    vec![sample(0.04, 300), sample(0.02, 280), sample(0.03, 310)],
                ),
                cell("cursor", vec![]),
            ],
        }
    }

    /// A result reads back equal to what was written, with every optional
    /// field present or null; the derived summaries are written beside the
    /// samples; a result of another schema or tool is refused.
    #[test]
    fn result_round_trips_through_json() {
        let full = result();
        let mut bare = result();
        let environment = &mut bare.environment;
        (environment.os, environment.cpu, environment.governor) = (None, None, None);
        (environment.pinned_cpus, environment.cpus_allowed) = (None, None);
        environment.build.source = None;
        environment.tools.clear();
        for result in [full, bare] {
            let text = result.to_json().to_pretty();
            let parsed = Value::parse(&text).unwrap();
            assert_eq!(RunResult::from_json(&parsed).unwrap(), result);
        }

        let value = result().to_json();
        let ascii = &value.get("cells").unwrap().as_array().unwrap()[0];
        let stats = ascii.get("stats").unwrap();
        let seconds = stats.get("seconds").unwrap();
        assert_eq!(seconds.get("median").unwrap().as_f64(), Some(0.03));
        assert_eq!(seconds.get("max").unwrap().as_f64(), Some(0.04));
        let rate = stats.get("mb_per_s").unwrap();
        assert_eq!(rate.get("max").unwrap().as_f64(), Some(16.777289 / 0.02));
        assert_eq!(rate.get("p95").unwrap().as_f64(), Some(16.777289 / 0.04));
        let reads = ascii.get("counters").unwrap().get("pty_reads").unwrap();
        assert_eq!(reads.get("median").unwrap().as_f64(), Some(300.0));
        let cursor = &value.get("cells").unwrap().as_array().unwrap()[1];
        assert_eq!(cursor.get("stats"), Some(&Value::Null));

        for (key, replacement) in [("schema", Value::from(2u64)), ("tool", "other".into())] {
            let Value::Object(mut entries) = result().to_json() else {
                unreachable!()
            };
            entries.iter_mut().find(|(name, _)| name == key).unwrap().1 = replacement;
            assert!(RunResult::from_json(&Value::Object(entries)).is_err());
        }
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let leap_day = UNIX_EPOCH + Duration::from_secs(951_782_400 + 86_399);
        assert_eq!(utc_timestamp(leap_day), "2000-02-29T23:59:59Z");
        let march = UNIX_EPOCH + Duration::from_secs(1_772_323_200);
        assert_eq!(utc_timestamp(march), "2026-03-01T00:00:00Z");
    }
}
