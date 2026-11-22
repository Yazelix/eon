//! Stream evidence; retain bounded numeric samples, never per-frame JSON trees.
use super::*;
use std::io::{BufRead, BufReader, Read};

const MAX_FRAMES: usize = 500_000;

#[derive(Clone, Copy)]
struct Frame {
    at: u64,
    revision: Option<u64>,
    first: bool,
    // CPU-labelled metric is elapsed host work, not process CPU accounting.
    times: [u64; 7],
    full: bool,
}
fn u64_field(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_u64)
        .ok_or(format!("missing/invalid frame {key}"))
}
fn nullable(row: &Value, key: &str) -> Result<Option<u64>> {
    match row.get(key) {
        Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or(format!("invalid frame {key}")),
        None => Err(format!("missing frame {key}")),
    }
}
fn record(row: &Value) -> Result<&str> {
    let Value::Object(fields) = row else {
        return Err("frame row is not an object".into());
    };
    if fields
        .iter()
        .enumerate()
        .any(|(i, (key, _))| fields[..i].iter().any(|(earlier, _)| earlier == key))
    {
        return Err("duplicate frame diagnostic key".into());
    }
    if u64_field(row, "v")? != 2 {
        return Err("unsupported frame log version".into());
    }
    row.get("kind")
        .and_then(Value::as_str)
        .ok_or("missing frame record kind".into())
}
fn frame(row: &Value) -> Result<Frame> {
    if record(row)? != "frame" || u64_field(row, "dropped")? != 0 {
        return Err("invalid frame record or dropped rows".into());
    }
    let at = u64_field(row, "present_ns")?;
    if nullable(row, "receive_ns")?.is_some_and(|n| n > at) {
        return Err("frame receive is after presentation".into());
    }
    let scene = u64_field(row, "scene_ns")?;
    let render = u64_field(row, "render_ns")?;
    let rebuild = u64_field(row, "rebuild_ns")?;
    let prepare = u64_field(row, "prepare_ns")?;
    let acquire = u64_field(row, "acquire_ns")?;
    let submit = u64_field(row, "submit_ns")?;
    if rebuild > render || prepare > render {
        return Err("frame subinterval exceeds render interval".into());
    }
    let elapsed = scene
        .checked_add(render)
        .and_then(|n| n.checked_add(submit))
        .ok_or("frame duration overflow")?;
    if elapsed > 180_000_000_000 || acquire > 180_000_000_000 {
        return Err("frame interval exceeds the sample deadline bound".into());
    }
    Ok(Frame {
        at,
        revision: nullable(row, "revision")?,
        first: false,
        times: [elapsed, scene, render, rebuild, prepare, acquire, submit],
        full: row
            .get("full_rebuild")
            .and_then(Value::as_bool)
            .ok_or("missing/invalid full_rebuild")?,
    })
}

fn distribution(values: &[f64]) -> Value {
    Summary::of(values).map_or(Value::Null, |s| {
        object([
            ("n", (s.n as u64).into()),
            ("unit", "ms".into()),
            ("p50", s.median.into()),
            ("p95", s.p95.into()),
            ("p99", (s.n >= 100).then_some(s.p99).into()),
            ("min", s.min.into()),
            ("max", s.max.into()),
        ])
    })
}
fn aggregate(frames: &[Frame]) -> Value {
    let mut fields = Vec::new();
    for (i, name) in [
        "frame_host",
        "scene",
        "render",
        "rebuild",
        "prepare",
        "acquire",
        "submit",
    ]
    .iter()
    .enumerate()
    {
        let values = frames
            .iter()
            .map(|f| f.times[i] as f64 / 1e6)
            .collect::<Vec<_>>();
        fields.push(((*name).into(), distribution(&values)));
    }
    let gaps = frames
        .windows(2)
        .filter(|f| !f[1].first)
        .map(|f| (f[1].at - f[0].at) as f64 / 1e6)
        .collect::<Vec<_>>();
    fields.push(("submit_gap".into(), distribution(&gaps)));
    let revisions = frames
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            f.revision.is_some() && (*i == 0 || f.first || frames[*i - 1].revision != f.revision)
        })
        .count();
    fields.extend([
        ("frames".into(), (frames.len() as u64).into()),
        ("revision_runs".into(), (revisions as u64).into()),
        (
            "frames_per_revision_run".into(),
            (revisions > 0)
                .then(|| frames.len() as f64 / revisions as f64)
                .into(),
        ),
        (
            "full_rebuilds".into(),
            (frames.iter().filter(|f| f.full).count() as u64).into(),
        ),
        (
            "full_rebuilds_per_frame".into(),
            (!frames.is_empty())
                .then(|| frames.iter().filter(|f| f.full).count() as f64 / frames.len() as f64)
                .into(),
        ),
    ]);
    Value::Object(fields)
}

pub(super) struct Window {
    pub path: PathBuf,
    pub pid: u32,
    pub revision: u64,
    pub completion_revision: Option<u64>,
    pub begin: u64,
    pub stop: u64,
    pub end: u64,
}
struct Stream {
    frames: Vec<Frame>,
    adapter: String,
    presentations: u64,
}
fn load_frames(window: &Window) -> Result<Stream> {
    if window.begin > window.stop || window.stop > window.end {
        return Err("invalid frame measurement window".into());
    }
    let mut reader = BufReader::new(
        fs::File::open(&window.path)
            .map_err(|e| format!("candidate lacks readable logger v2: {e}"))?,
    );
    let mut line = String::new();
    let mut measured = Vec::new();
    let (mut rows, mut last) = (0u64, 0);
    let mut adapter = None;
    let mut ended = false;
    let mut setup_revision = None;
    loop {
        line.clear();
        let n = Read::by_ref(&mut reader)
            .take(4097)
            .read_line(&mut line)
            .map_err(error)?;
        if n == 0 {
            break;
        }
        if n > 4096 || !line.ends_with('\n') {
            return Err("oversize/truncated frame row".into());
        }
        let row = Value::parse(&line)?;
        if ended {
            return Err("records after frame log footer".into());
        }
        match record(&row)? {
            "start" if adapter.is_none() && rows == 0 => {
                if u64_field(&row, "pid")? != u64::from(window.pid) {
                    return Err("frame log PID does not match the private Venus process".into());
                }
                adapter = Some(
                    row.get("adapter")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .ok_or("missing frame adapter identity")?
                        .to_owned(),
                );
            }
            "frame" if adapter.is_some() => {
                let mut f = frame(&row)?;
                if f.at <= last {
                    return Err("non-increasing frame timestamps".into());
                }
                last = f.at;
                rows += 1;
                if rows > MAX_FRAMES as u64 {
                    return Err("sample exceeds 500000-frame memory budget".into());
                }
                if f.at < window.begin {
                    setup_revision = f.revision;
                }
                if f.at >= window.begin && f.at < window.end {
                    if f.revision.is_none() {
                        return Err("measured presentation lacks a terminal revision".into());
                    }
                    f.first = measured.is_empty();
                    measured.push(f);
                }
            }
            "end" if adapter.is_some() => {
                let total = u64_field(&row, "presentations")?;
                if u64_field(&row, "dropped")? != 0 || rows != total || rows == 0 {
                    return Err(format!(
                        "incomplete frame diagnostics: {rows} rows for {total} presentations"
                    ));
                }
                ended = true;
            }
            _ => return Err("unexpected or misordered frame log record".into()),
        }
    }
    if !ended {
        return Err("missing frame log start or clean-shutdown footer".into());
    }
    if setup_revision.is_none_or(|r| r < window.revision) {
        return Err("fixture revision was not presented before the measurement window".into());
    }
    if window.completion_revision.is_some_and(|target| {
        !measured
            .iter()
            .any(|f| f.revision.is_some_and(|r| r >= target))
    }) {
        return Err(
            "throughput completion revision was not presented within the measurement window".into(),
        );
    }
    Ok(Stream {
        frames: measured,
        adapter: adapter.ok_or("missing frame log start")?,
        presentations: rows,
    })
}
pub(super) fn timings(window: &Window, require_frames: bool) -> Result<Value> {
    let stream = load_frames(window)?;
    let measured = stream.frames;
    if require_frames && measured.len() < 2 {
        return Err("workload produced fewer than two measured frames".into());
    }
    let split = measured.partition_point(|f| f.at < window.stop);
    Ok(object([
        ("whole", aggregate(&measured)),
        ("input", aggregate(&measured[..split])),
        ("tail", aggregate(&measured[split..])),
        ("all_presented_rows", stream.presentations.into()),
        ("adapter", stream.adapter.into()),
    ]))
}

pub(super) fn pooled(windows: &[Window]) -> Result<Value> {
    let mut frames = Vec::new();
    for window in windows {
        let sample = load_frames(window)?.frames;
        if frames.len() + sample.len() > MAX_FRAMES {
            return Err("pooled cell exceeds 500000-frame memory budget".into());
        }
        frames.extend(sample);
    }
    Ok(object([
        (
            "scope",
            "pooled per-frame observations; gaps exclude boundaries between runs".into(),
        ),
        ("runs", (windows.len() as u64).into()),
        ("frames", aggregate(&frames)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn throughput_completion_after_window_must_not_escape_statistics() {
        let dir = std::env::temp_dir().join(format!(
            "frames-completion-{}-{}",
            std::process::id(),
            environment::monotonic_ns()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("frames.jsonl");
        let row = |at, revision, render| {
            format!(
                "{{\"v\":2,\"kind\":\"frame\",\"revision\":{revision},\"receive_ns\":null,\"present_ns\":{at},\"scene_ns\":2,\"render_ns\":{render},\"rebuild_ns\":4,\"prepare_ns\":3,\"acquire_ns\":5,\"submit_ns\":1,\"full_rebuild\":true,\"dropped\":0}}\n"
            )
        };
        let prefix = "{\"v\":2,\"kind\":\"start\",\"pid\":3,\"adapter\":\"software=true AdapterInfo { backend: Vulkan }\"}\n".to_owned()
            + &row(50_000_000,7,1_000_000) + &row(120_000_000,8,2_000_000) + &row(140_000_000,9,2_000_000);
        let footer = "{\"v\":2,\"kind\":\"end\",\"presentations\":4,\"dropped\":0}\n";
        let window = Window {
            path: path.clone(),
            pid: 3,
            revision: 7,
            completion_revision: Some(10),
            begin: 100_000_000,
            stop: 250_000_000,
            end: 300_000_000,
        };
        fs::write(
            &path,
            prefix.clone() + &row(400_000_000, 10, 150_000_000) + footer,
        )
        .unwrap();
        assert!(
            timings(&window, true).is_err(),
            "late final revision was silently excluded"
        );
        assert!(pooled(std::slice::from_ref(&window)).is_err());
        fs::write(
            &path,
            prefix.clone() + &row(300_000_000, 10, 150_000_000) + footer,
        )
        .unwrap();
        assert!(timings(&window, true).is_err(), "end boundary is exclusive");
        fs::write(
            &path,
            prefix.clone() + &row(299_000_000, 9, 150_000_000) + footer,
        )
        .unwrap();
        assert!(
            timings(&window, true).is_err(),
            "completion revision never presented"
        );
        fs::write(&path, prefix + &row(299_000_000, 10, 150_000_000) + footer).unwrap();
        let measured = timings(&window, true).unwrap();
        assert_eq!(
            measured
                .get("whole")
                .unwrap()
                .get("frames")
                .unwrap()
                .as_u64(),
            Some(3)
        );
        assert!(
            measured
                .get("whole")
                .unwrap()
                .get("frame_host")
                .unwrap()
                .get("max")
                .unwrap()
                .as_f64()
                .unwrap()
                > 150.0
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn known_frame_cost_does_not_double_count_and_small_p99_is_unreported() {
        let f = frame(&Value::parse(r#"{"v":2,"kind":"frame","revision":7,"receive_ns":null,"present_ns":99,"scene_ns":2,"render_ns":10,"rebuild_ns":4,"prepare_ns":3,"acquire_ns":5,"submit_ns":1,"full_rebuild":true,"dropped":0}"#).unwrap()).unwrap();
        assert_eq!(f.times[0], 13);
        let stats = aggregate(&[f]);
        assert_eq!(
            stats.get("full_rebuilds_per_frame").unwrap().as_f64(),
            Some(1.0)
        );
        assert_eq!(
            stats.get("frame_host").unwrap().get("p99"),
            Some(&Value::Null)
        );
        assert_eq!(aggregate(&[]).get("frame_host"), Some(&Value::Null));
    }
    #[test]
    fn missing_malformed_truncated_and_tail_missing_logs_cannot_publish_metrics() {
        let dir = std::env::temp_dir().join(format!(
            "frames-diagnostics-{}-{}",
            std::process::id(),
            environment::monotonic_ns()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("frames.jsonl");
        let row = |at| {
            format!(
                "{{\"v\":2,\"kind\":\"frame\",\"revision\":7,\"receive_ns\":null,\"present_ns\":{at},\"scene_ns\":2,\"render_ns\":10,\"rebuild_ns\":4,\"prepare_ns\":3,\"acquire_ns\":5,\"submit_ns\":1,\"full_rebuild\":true,\"dropped\":0}}\n"
            )
        };
        let start = "{\"v\":2,\"kind\":\"start\",\"pid\":3,\"adapter\":\"software=true AdapterInfo { backend: Vulkan }\"}\n";
        let end = "{\"v\":2,\"kind\":\"end\",\"presentations\":3,\"dropped\":0}\n";
        let window = Window {
            path: path.clone(),
            pid: 3,
            revision: 7,
            completion_revision: None,
            begin: 10,
            stop: 30,
            end: 40,
        };
        assert!(timings(&window, true).is_err());
        let good = start.to_owned() + &row(5) + &row(10) + &row(20) + end;
        fs::write(&path, &good).unwrap();
        assert!(timings(&window, true).is_ok());
        assert!(timings(&Window { pid: 4, ..window }, true).is_err());
        let window = Window {
            path: path.clone(),
            pid: 3,
            revision: 8,
            completion_revision: None,
            begin: 10,
            stop: 30,
            end: 40,
        };
        assert!(timings(&window, true).is_err());
        let window = Window {
            revision: 7,
            begin: 21,
            ..window
        };
        assert_eq!(
            timings(&window, false)
                .unwrap()
                .get("whole")
                .unwrap()
                .get("frames")
                .unwrap()
                .as_u64(),
            Some(0)
        );
        for bad in [
            good.trim_end().to_owned(),
            good.replace("\"dropped\":0", "\"dropped\":1"),
            good.replace("\"v\":2", "\"v\":1"),
            good.replace("\"v\":2", "\"v\":2,\"v\":2"),
            good.replace("\"presentations\":3", "\"presentations\":4"),
            good.replace(end, ""),
            good.replace(start, ""),
            good.clone() + &row(30),
            start.to_owned() + &row(5) + &row(10) + &row(10) + end,
            good.replace("\"render_ns\":10", "\"render_ns\":0"),
            "".into(),
        ] {
            fs::write(&path, bad).unwrap();
            assert!(timings(&window, false).is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pooling_never_invents_a_gap_between_runs() {
        let f = |at, first| Frame {
            at,
            revision: Some(7),
            first,
            times: [1; 7],
            full: false,
        };
        let stats = aggregate(&[
            f(10, true),
            f(20, false),
            f(1_000_000, true),
            f(1_000_010, false),
        ]);
        assert_eq!(
            stats.get("submit_gap").unwrap().get("n").unwrap().as_u64(),
            Some(2)
        );
        assert_eq!(stats.get("revision_runs").unwrap().as_u64(), Some(2));
    }
}
