//! One fresh application window, excluding the held benchmark workload tree.
use super::*;

fn descendants(all: &[Process], root: u32) -> Vec<Process> {
    let mut pids = std::collections::BTreeSet::from([root]);
    loop {
        let before = pids.len();
        for p in all {
            if pids.contains(&p.parent) {
                pids.insert(p.pid);
            }
        }
        if pids.len() == before {
            break;
        }
    }
    all.iter()
        .filter(|p| pids.contains(&p.pid))
        .cloned()
        .collect()
}

fn snapshot(root: &Process) -> Result<Vec<Process>> {
    let mut all = fs::read_dir("/proc")
        .map_err(error)?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| Process::at(pid).ok())
        .collect::<Vec<_>>();
    all.sort_by_key(|p| p.pid);
    let tree = descendants(&all, root.pid);
    if !root.alive()
        || !tree
            .iter()
            .any(|p| p.pid == root.pid && p.start == root.start)
    {
        return Err("application root exited/reused".into());
    }
    validate_set(&tree)?;
    Ok(tree)
}

fn is_venus(exe: &Path, eon: bool, included: bool) -> bool {
    eon && included
        && exe
            .file_name()
            .is_some_and(|n| n == ".yazelix-venus-wrapped" || n == "yazelix-venus")
}

fn validate_venus_count(eon: bool, count: usize) -> Result {
    if eon && count != 1 {
        return Err(format!("expected exactly one Venus, observed {count}"));
    }
    Ok(())
}

pub(crate) fn measure(root: u32, held: u32, eon: bool, dir: &Path) -> Result<Value> {
    let root = Process::at(root)?;
    let held = Process::at(held)?;
    let before = snapshot(&root)?;
    if root.pid == held.pid
        || !before
            .iter()
            .any(|p| p.pid == held.pid && p.start == held.start)
    {
        return Err("held workload is not inside the application tree".into());
    }
    let excluded = descendants(&before, held.pid);
    let mut total = [0u64; 3];
    let mut venus = [0u64; 3];
    let mut venus_count = 0;
    let mut rows = Vec::new();
    let start_load = environment::load()?;
    let started = Instant::now();
    for p in &before {
        let included = !excluded.iter().any(|child| child.pid == p.pid);
        let exe = fs::read_link(format!("/proc/{}/exe", p.pid)).map_err(error)?;
        let is_venus = is_venus(&exe, eon, included);
        venus_count += usize::from(is_venus);
        let rollup = read(&PathBuf::from(format!("/proc/{}/smaps_rollup", p.pid)))?;
        for (index, (_, field)) in METRICS.iter().enumerate() {
            let bytes = metric(&rollup, field)?;
            if included {
                total[index] = total[index]
                    .checked_add(bytes)
                    .ok_or("memory sum overflow")?;
            }
            if is_venus {
                venus[index] = bytes;
            }
        }
        rows.push(object(vec![
            ("pid", u64::from(p.pid).into()),
            ("start_ticks", p.start.into()),
            ("parent_pid", u64::from(p.parent).into()),
            ("included", included.into()),
            (
                "role",
                if !included {
                    "excluded-workload"
                } else if is_venus {
                    "venus"
                } else {
                    "application"
                }
                .into(),
            ),
            ("exe", exe.display().to_string().into()),
            (
                "cmdline",
                String::from_utf8_lossy(
                    &fs::read(format!("/proc/{}/cmdline", p.pid)).map_err(error)?,
                )
                .replace('\0', " ")
                .into(),
            ),
            ("rollup", rollup.into()),
        ]));
    }
    let after = snapshot(&root)?;
    if !held.alive()
        || before
            .iter()
            .map(|p| (p.pid, p.start))
            .ne(after.iter().map(|p| (p.pid, p.start)))
    {
        return Err("application process set changed during memory collection".into());
    }
    validate_venus_count(eon, venus_count)?;
    let mut fields = vec![
        ("collection_seconds", started.elapsed().as_secs_f64().into()),
        ("load_start", start_load.into()),
        ("load_end", environment::load()?.into()),
        ("windows", 1u64.into()),
        ("processes", Value::Array(rows)),
    ];
    for (index, (name, _)) in METRICS.iter().enumerate() {
        fields.push((*name, total[index].into()));
    }
    let mut record = object(fields);
    if eon && let Value::Object(fields) = &mut record {
        fields.extend(
            METRICS
                .iter()
                .enumerate()
                .map(|(i, (name, _))| (format!("venus_{name}"), venus[i].into())),
        );
    }
    fs::write(dir.join("memory.json"), record.to_pretty()).map_err(error)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "linux")]
    fn live_rollups_reconstruct_the_included_application_total() {
        use std::io::BufRead;
        let dir = std::env::temp_dir().join(format!("window-memory-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & echo $!; wait"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let held = line.trim().parse().unwrap();
        let held_process = Process::at(held).unwrap();
        let result = measure(child.id(), held, false, &dir);
        let missing_venus = measure(child.id(), held, true, &dir);
        let _ = child.kill();
        let _ = child.wait();
        held_process.signal(libc::SIGKILL);
        assert_eq!(
            missing_venus.unwrap_err(),
            "expected exactly one Venus, observed 0"
        );
        let record = result.unwrap();
        let rows = record.get("processes").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let included = rows
            .iter()
            .filter(|p| p.get("included").and_then(Value::as_bool) == Some(true))
            .collect::<Vec<_>>();
        assert_eq!(included.len(), 1);
        let rollup = included[0].get("rollup").unwrap().as_str().unwrap();
        for (name, field) in METRICS {
            assert_eq!(
                record.get(name).and_then(Value::as_u64),
                Some(metric(rollup, field).unwrap())
            );
        }
        assert!(record.get("venus_pss_bytes").is_none());
    }
    #[test]
    fn venus_attribution_requires_one_included_exact_executable() {
        for name in [".yazelix-venus-wrapped", "yazelix-venus"] {
            let exe = Path::new("/nix/store/candidate/bin").join(name);
            assert!(is_venus(&exe, true, true));
            assert!(
                !is_venus(&exe, true, false),
                "excluded workload must never count as Venus"
            );
            assert!(!is_venus(&exe, false, true));
        }
        for name in ["venus", ".yazelix-venus-wrapped-extra", "not-yazelix-venus"] {
            assert!(!is_venus(Path::new(name), true, true));
        }
        assert!(validate_venus_count(true, 1).is_ok());
        for count in [0, 2] {
            assert_eq!(
                validate_venus_count(true, count).unwrap_err(),
                format!("expected exactly one Venus, observed {count}")
            );
        }
        assert!(validate_venus_count(false, 0).is_ok());
        let count = [true, false]
            .into_iter()
            .filter(|&included| is_venus(Path::new("yazelix-venus"), true, included))
            .count();
        assert_eq!(count, 1);
        assert!(validate_venus_count(true, count).is_ok());
    }
    #[test]
    fn window_tree_keeps_application_support_and_excludes_only_workload_descendants() {
        let process = |pid, parent| Process {
            pid,
            parent,
            start: 1,
        };
        let all = vec![
            process(7, 6),
            process(1, 0),
            process(6, 3),
            process(8, 1),
            process(3, 2),
            process(4, 3),
            process(2, 1),
            process(5, 4),
        ];
        let tree = descendants(&all, 3);
        let workload = descendants(&tree, 4);
        let included = tree
            .iter()
            .filter(|p| !workload.iter().any(|w| w.pid == p.pid))
            .map(|p| p.pid)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(included, std::collections::BTreeSet::from([3, 6, 7]));
    }
}
