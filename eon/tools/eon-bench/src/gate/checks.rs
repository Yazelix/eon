use super::*;
use crate::result::{Build, Cell, Environment, Grid, Sample};

fn fixture(suite: &str) -> Value {
    let cell = |workload: &str, layer: &str, subject: &str| {
        let corpus = crate::corpus::find(workload);
        Cell {
            workload: workload.into(),
            layer: layer.into(),
            subject: subject.into(),
            subject_version: "test".into(),
            corpus_sha256: corpus.map_or(String::new(), |c| c.sha256.into()),
            input_bytes: corpus.map_or(0, |c| c.length),
            samples: (1..=10)
                .map(|n| Sample {
                    seconds: f64::from(n),
                    load: 0.5,
                    counters: Vec::new(),
                })
                .collect(),
        }
    };
    let cells = if suite == "orbit" {
        crate::corpus::CORPORA
            .iter()
            .flat_map(|c| {
                [
                    cell(c.id, "orbit-detached", "yazelix-orbit"),
                    cell(c.id, "orbit-attached", "yazelix-orbit"),
                ]
            })
            .collect()
    } else {
        std::iter::once("cat64".to_owned())
            .chain(
                crate::kitten::BENCHMARKS
                    .iter()
                    .map(|n| format!("kitten-{n}")),
            )
            .map(|name| cell(&name, "render-enabled-completion", "eon"))
            .collect()
    };
    let mut raw = RunResult {
        suite: suite.into(),
        tool_version: "test".into(),
        quick: false,
        corpus_version: crate::corpus::CORPUS_VERSION as u64,
        warmups: 2,
        samples: 10,
        started_at: String::new(),
        finished_at: String::new(),
        cells,
        environment: Environment {
            os: Some("test OS".into()),
            cpu: Some("test CPU".into()),
            cpus_online: 1,
            pinned_cpus: Some(vec![0]),
            cpus_allowed: Some(vec![0]),
            governor: Some("test governor".into()),
            kernel: "test kernel".into(),
            load_start: 0.5,
            load_end: 0.5,
            max_load: 1.0,
            grid: Grid { cols: 80, rows: 24 },
            tools: [
                "font-file",
                "font-px",
                "scrollback",
                "kitten",
                "kitten-repetitions",
            ]
            .into_iter()
            .map(|key| (key.into(), "test".into()))
            .collect(),
            build: Build {
                path: "/test/bench".into(),
                sha256: "test hash".into(),
                profile: "release".into(),
                source: None,
            },
        },
    }
    .to_json();
    if suite == "terminals"
        && let Value::Object(fields) = &mut raw
    {
        fields.extend(
            [
                "requested_renderer",
                "font_family",
                "fontconfig",
                "product_fontconfig_mount",
            ]
            .into_iter()
            .map(|k| (k.into(), "test".into())),
        );
        fields.push(("result".into(), "pass".into()));
        fields.push((
            "adapter_preflight".into(),
            object([
                ("result", "pass".into()),
                ("compositor", "Sway headless".into()),
                ("compositor_version", "test Sway".into()),
                ("compositor_renderer", "pixman".into()),
                ("vulkan_icd", "/test/icd.json".into()),
            ]),
        ));
    }
    raw
}
fn at<'a>(value: &'a mut Value, path: &[&str]) -> &'a mut Value {
    let Some((key, tail)) = path.split_first() else {
        return value;
    };
    let next = match value {
        Value::Object(fields) => &mut fields.iter_mut().find(|(name, _)| name == key).unwrap().1,
        Value::Array(values) => &mut values[key.parse::<usize>().unwrap()],
        _ => panic!("invalid test path"),
    };
    at(next, tail)
}
#[test]
fn snapshot_recomputes_complete_samples_and_keeps_candidate_artifacts() {
    let mut runs = [fixture("orbit"), fixture("terminals")];
    *at(&mut runs[0], &["cells", "0", "stats", "seconds", "median"]) = 999.0.into();
    let result = snapshot(&runs, "test machine").unwrap();
    let measured = cells(&result).unwrap();
    assert_eq!(measured.len(), 18);
    assert!(measured.values().all(|t| *t
        == Timing {
            p50: 5.5,
            p95: 10.0
        }));
    let artifacts = result.get("artifacts").unwrap().as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(
        text(field(&artifacts[0], "build").unwrap(), "sha256").unwrap(),
        "test hash"
    );
}
#[test]
fn snapshot_rejects_partial_settings_bad_samples_and_failed_suites() {
    let original = [fixture("orbit"), fixture("terminals")];
    for (suite, path, bad) in [
        (0, vec!["quick"], true.into()),
        (0, vec!["samples"], 9u64.into()),
        (0, vec!["warmups"], 1u64.into()),
        (0, vec!["environment", "build", "profile"], "debug".into()),
        (0, vec!["environment", "grid", "cols"], 81u64.into()),
        (
            0,
            vec!["cells", "0", "samples", "0", "seconds"],
            f64::NAN.into(),
        ),
        (0, vec!["cells", "0", "samples", "0", "seconds"], 0.0.into()),
        (0, vec!["cells", "0", "samples", "0", "load"], 2.0.into()),
        (0, vec!["cells", "0", "workload"], "unexpected".into()),
        (1, vec!["result"], "fail".into()),
        (1, vec!["adapter_preflight", "result"], "fail".into()),
    ] {
        let mut runs = original.clone();
        *at(&mut runs[suite], &path) = bad;
        assert!(
            snapshot(&runs, "test machine").is_err(),
            "accepted {path:?}"
        );
    }
    let mut runs = original.clone();
    if let Value::Array(cells) = at(&mut runs[0], &["cells"]) {
        cells.pop();
    }
    assert!(snapshot(&runs, "test machine").is_err());
    let mut runs = original.clone();
    if let Value::Array(samples) = at(&mut runs[0], &["cells", "0", "samples"]) {
        samples.pop();
    }
    assert!(snapshot(&runs, "test machine").is_err());
    assert!(snapshot(&original[..1], "test machine").is_err());
}
#[test]
fn conditions_refuse_changed_vulkan_icd_compositor_or_machine_before_timing() {
    let snapshot = snapshot(&[fixture("orbit"), fixture("terminals")], "test machine").unwrap();
    let expected = field(&snapshot, "conditions").unwrap();
    check_conditions(expected, expected).unwrap();
    for path in [
        vec!["terminals", "adapter", "vulkan_icd"],
        vec!["terminals", "adapter", "compositor_version"],
        vec!["orbit", "kernel"],
    ] {
        let mut changed = expected.clone();
        *at(&mut changed, &path) = "different".into();
        assert!(
            check_conditions(&changed, expected)
                .unwrap_err()
                .contains("conditions differ")
        );
    }
}
#[test]
fn recording_refuses_existing_baselines_and_unwritable_directory_routes_early() {
    let dir = std::env::temp_dir().join(format!("gate-destination-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let baseline = dir.join("baseline.json");
    preflight::record_destination(&baseline).unwrap();
    assert!(!baseline.exists());
    fs::write(&baseline, "accepted baseline").unwrap();
    assert!(
        preflight::record_destination(&baseline)
            .unwrap_err()
            .contains("exists")
    );
    assert_eq!(fs::read_to_string(&baseline).unwrap(), "accepted baseline");
    assert!(
        preflight::record_destination(&dir.join("missing/baseline.json"))
            .unwrap_err()
            .contains("directory")
    );
    let link = dir.join("dangling");
    std::os::unix::fs::symlink(dir.join("absent"), &link).unwrap();
    assert!(
        preflight::record_destination(&link)
            .unwrap_err()
            .contains("exists")
    );
    fs::remove_dir_all(dir).unwrap();
}
