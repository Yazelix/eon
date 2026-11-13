//! Dependency-context fingerprints and explicit maintenance of audited exceptions.
use super::*;
use std::io::Write;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Context {
    pub producer: String,
    target: String,
    roots: BTreeSet<String>,
    default_features: bool,
    features: BTreeSet<String>,
    build_flags: Vec<String>,
    build_command: String,
    toolchain: BTreeMap<String, String>,
    pub exceptions: PathBuf,
    pub lock_file: PathBuf,
    pub evidence_documents: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Binary {
    pub path: PathBuf,
    pub packages: Vec<serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Snapshot {
    pub context: Context,
    pub graph_sha256: String,
    pub shipped: BTreeSet<(String, String)>,
    pub binaries: Vec<Binary>,
    missing: BTreeMap<String, BTreeSet<(String, String)>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Group {
    root: String,
    graph_sha256: String,
    reason: String,
    evidence: String,
    packages: BTreeSet<(String, String)>,
}

// Cargo validates the lockfile first; normalize only canonical local-version records.
fn lock_graph(lock: &str) -> Result<(String, BTreeSet<(String, String)>)> {
    let blocks: Vec<_> = lock.split("\n[[package]]\n").collect();
    if blocks.len() < 2 {
        return Err("Cargo.lock lacks canonical package stanzas".into());
    }
    let field = |block: &str, key: &str| -> Result<String> {
        let value = block
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .ok_or("Cargo.lock package lacks name/version")?;
        Ok(serde_json::from_str(value)?)
    };
    let mut local = BTreeSet::new();
    for block in &blocks[1..] {
        let identity = (field(block, "name = ")?, field(block, "version = ")?);
        if !block.lines().any(|line| line.starts_with("source = ")) {
            local.insert(identity);
        }
    }
    let mut normalized = Vec::new();
    normalized.push(blocks[0].to_owned());
    for block in &blocks[1..] {
        let is_local = local.contains(&(field(block, "name = ")?, field(block, "version = ")?));
        let mut lines = Vec::new();
        for line in block.lines() {
            if is_local && line.starts_with("version = ") {
                lines.push("version = \"<local>\"".to_owned());
                continue;
            }
            let trimmed = line.trim();
            let value = trimmed.strip_suffix(',').unwrap_or(trimmed);
            if value.starts_with('"') && value.ends_with('"') {
                let dependency: String = serde_json::from_str(value)?;
                if let Some((name, _)) = local
                    .iter()
                    .find(|(name, version)| dependency == format!("{name} {version}"))
                {
                    lines.push(format!(
                        "{}\"{name} <local>\"{}",
                        &line[..line.len() - line.trim_start().len()],
                        if trimmed.ends_with(',') { "," } else { "" }
                    ));
                    continue;
                }
            }
            lines.push(line.to_owned());
        }
        normalized.push(lines.join("\n"));
    }
    Ok((normalized.join("\n[[package]]\n"), local))
}

fn graph(
    context: &Context,
    lock: &str,
    shipped: &BTreeSet<(String, String)>,
    native: &[NativePackage],
    binaries: &[Binary],
) -> Result<serde_json::Value> {
    if context.producer.is_empty() || context.target.is_empty() || context.roots.is_empty() {
        return Err("incomplete audit build context".into());
    }
    let (lock, local) = lock_graph(lock)?;
    let identity = |name: &str, version: &str| {
        (
            name.to_owned(),
            if local.contains(&(name.to_owned(), version.to_owned())) {
                "<local>".to_owned()
            } else {
                version.to_owned()
            },
        )
    };
    let runtime: BTreeSet<_> = shipped.iter().map(|(n, v)| identity(n, v)).collect();
    let native: BTreeSet<_> = native
        .iter()
        .map(|p| {
            // Cargo may print a (proc-macro) label before a local source path.
            let origin = if local.contains(&(p.name.clone(), p.version.clone())) {
                "local"
            } else {
                p.origin
                    .split_once(" (")
                    .map(|(_, source)| source.trim_end_matches(')'))
                    .unwrap_or("registry")
            };
            serde_json::to_string(&serde_json::json!({
                "package": identity(&p.name, &p.version),
                "source": origin,
                "features": p.features
            }))
        })
        .collect::<std::result::Result<_, _>>()?;
    let mut audits = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for binary in binaries {
        let mut packages = BTreeSet::new();
        let mut found = Vec::new();
        for p in &binary.packages {
            let (name, version) = cargo_identity(p)?;
            let kind = p
                .get("kind")
                .map(|kind| kind.as_str().ok_or("invalid Rust audit kind"))
                .transpose()?
                .unwrap_or("runtime");
            if !["runtime", "build"].contains(&kind) {
                return Err(format!("unknown Rust audit kind {kind}").into());
            }
            if p["root"].as_bool() == Some(true) {
                if kind != "runtime" {
                    return Err("Rust audit root is not runtime-kind".into());
                }
                found.push(name.clone());
            }
            packages.insert(serde_json::to_string(&serde_json::json!({"package":identity(&name,&version),"kind":kind,"source":p["source"],"root":p["root"].as_bool().unwrap_or(false)}))?);
        }
        if found.len() != 1 {
            return Err(format!(
                "{}: expected exactly one Rust audit root",
                binary.path.display()
            )
            .into());
        }
        roots.insert(found[0].clone());
        audits.insert(serde_json::to_string(&(found[0].clone(), packages))?);
    }
    if roots != context.roots {
        return Err(format!("audited roots differ from producer roots: {roots:?}").into());
    }
    let mut context = serde_json::to_value(context)?;
    for key in ["evidence_documents", "exceptions", "lock_file"] {
        context.as_object_mut().unwrap().remove(key);
    }
    Ok(
        serde_json::json!({"schema":1,"context":context,"lock":lock,"runtime":runtime,"native":native,"audits":audits}),
    )
}

pub(super) fn inspect(
    context: &Path,
    lock: &Path,
    binaries: &Path,
    tree: &Path,
    native_tree: &Path,
    output: &Path,
) -> Result<Snapshot> {
    let context: Context = serde_json::from_slice(&fs::read(context)?)?;
    let lock = fs::read_to_string(lock)?;
    let tree = fs::read_to_string(tree)?;
    let native_tree = fs::read_to_string(native_tree)?;
    let shipped = runtime_crates(&tree)?;
    let native = native_packages(&native_tree)?;
    let mut files = Vec::new();
    tree_files(binaries, &mut files)?;
    let mut binaries = Vec::new();
    for path in files {
        if is_elf(&path)? {
            binaries.push(Binary {
                packages: audit_packages(&path)?,
                path,
            });
        }
    }
    let snapshot = snapshot(context, &lock, shipped, &native, binaries, output)?;
    fs::write(
        output.join("context.json"),
        serde_json::to_vec_pretty(&snapshot.context)?,
    )?;
    fs::write(output.join("Cargo.lock"), lock)?;
    fs::write(output.join("runtime-crates.txt"), tree)?;
    fs::write(output.join("native-packages.txt"), native_tree)?;
    Ok(snapshot)
}

fn snapshot(
    context: Context,
    lock: &str,
    shipped: BTreeSet<(String, String)>,
    native: &[NativePackage],
    binaries: Vec<Binary>,
    output: &Path,
) -> Result<Snapshot> {
    let value = graph(&context, lock, &shipped, native, &binaries)?;
    fs::create_dir(output)?;
    let graph_path = output.join("graph.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&value)?)?;
    let mut missing: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for binary in &binaries {
        let root = binary
            .packages
            .iter()
            .find(|p| p["root"].as_bool() == Some(true))
            .ok_or("missing root")?;
        let packages = binary
            .packages
            .iter()
            .filter(|p| p.get("kind").is_none_or(|kind| kind == "runtime"))
            .map(cargo_identity)
            .collect::<Result<BTreeSet<_>>>()?;
        let absent = packages
            .difference(&shipped)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !absent.is_empty() {
            missing
                .entry(cargo_identity(root)?.0)
                .or_default()
                .extend(absent);
        }
    }
    let snapshot = Snapshot {
        context,
        graph_sha256: digest(&graph_path)?,
        shipped,
        binaries,
        missing,
    };
    fs::write(
        output.join("observation.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    Ok(snapshot)
}

pub(super) fn check_evidence(reference: &str, documents: &BTreeMap<String, PathBuf>) -> Result<()> {
    let (file, anchor) = reference
        .split_once('#')
        .ok_or("audit evidence needs a document and anchor")?;
    let path = documents
        .get(file)
        .ok_or_else(|| format!("unknown audit evidence document: {file}"))?;
    if anchor.is_empty()
        || !fs::read_to_string(path)?
            .lines()
            .any(|line| line.trim_end() == format!("## {anchor}"))
    {
        return Err(format!("missing audit evidence anchor: {reference}").into());
    }
    Ok(())
}

pub(super) fn propose(
    exceptions: &Path,
    full: &Path,
    fallback: &Path,
    output: &Path,
) -> Result<()> {
    let mut groups: BTreeMap<String, Vec<Group>> = serde_json::from_slice(&fs::read(exceptions)?)?;
    let mut snapshots = BTreeMap::new();
    let mut previous_producers = None;
    for input in [full, fallback] {
        let input: Inputs = serde_json::from_slice(&fs::read(input)?)?;
        let mut producers = BTreeSet::new();
        for report in input.entries.iter().flat_map(|e| &e.cargo_reports) {
            let path = report
                .parent()
                .ok_or("Cargo report lacks parent")?
                .join("audit/observation.json");
            let snapshot: Snapshot = serde_json::from_slice(
                &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            )?;
            producers.insert(snapshot.context.producer.clone());
            if let Some(previous) = snapshots.insert(snapshot.context.producer.clone(), snapshot) {
                let current = &snapshots[&previous.context.producer];
                if previous.graph_sha256 != current.graph_sha256
                    || previous.missing != current.missing
                {
                    return Err(format!(
                        "variants have different audit contexts for {}",
                        current.context.producer
                    )
                    .into());
                }
            }
        }
        if let Some(previous) = previous_producers.replace(producers.clone())
            && previous != producers
        {
            return Err("variants have different Rust producer sets".into());
        }
    }
    for (producer, snapshot) in &snapshots {
        let mut expected: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for group in groups.get(producer).into_iter().flatten() {
            expected
                .entry(group.root.clone())
                .or_default()
                .extend(group.packages.iter().cloned());
        }
        if expected != snapshot.missing {
            return Err(format!(
                "{producer}: changed exception identities need fresh evidence; refusing renewal"
            )
            .into());
        }
        for group in groups.get_mut(producer).into_iter().flatten() {
            check_evidence(&group.evidence, &snapshot.context.evidence_documents)?;
            group.graph_sha256 = snapshot.graph_sha256.clone();
        }
    }
    if groups
        .keys()
        .any(|producer| !snapshots.contains_key(producer))
    {
        return Err("exception producer lacks an observation".into());
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&groups)?)?;
    Ok(())
}

pub(super) fn prove_omission(
    observation: &Path,
    exceptions: &Path,
    name: &str,
    version: &str,
) -> Result<()> {
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(observation)?)?;
    let groups: BTreeMap<String, Vec<Group>> = serde_json::from_slice(&fs::read(exceptions)?)?;
    let exceptions = groups
        .get(&snapshot.context.producer)
        .into_iter()
        .flatten()
        .flat_map(|g| {
            g.packages.iter().map(|p| AuditException {
                root: g.root.clone(),
                package: p.clone(),
                graph_sha256: g.graph_sha256.clone(),
                reason: g.reason.clone(),
                evidence: g.evidence.clone(),
            })
        })
        .collect::<Vec<_>>();
    binary_crates(&snapshot, &exceptions)?;
    let omitted = (name.to_owned(), version.to_owned());
    if !snapshot.shipped.contains(&omitted) {
        return Err("probe package is not covered by the inventory".into());
    }
    let mut shipped = snapshot.shipped.clone();
    shipped.remove(&omitted);
    for binary in &snapshot.binaries {
        if audit_packages(&binary.path)? != binary.packages {
            return Err("observation differs from the current binary".into());
        }
        let root = binary
            .packages
            .iter()
            .find(|p| p["root"].as_bool() == Some(true))
            .ok_or("missing root")?;
        if cargo_identity(root)? == omitted {
            continue;
        }
        if binary.packages.iter().any(|p| {
            p.get("kind").is_none_or(|kind| kind == "runtime")
                && cargo_identity(p).is_ok_and(|p| p == omitted)
        }) {
            if !shipped.contains(&cargo_identity(root)?) {
                return Err("probe unexpectedly removed the root".into());
            }
            let error = check_binary_crates(
                &binary.path,
                &binary.packages,
                &shipped,
                &exceptions,
                &mut BTreeSet::new(),
                &snapshot.graph_sha256,
            )
            .err()
            .ok_or("omitted audited dependency was accepted")?;
            if !error.to_string().contains(name) {
                return Err(error);
            }
            println!(
                "root-only baseline accepts the retained root; audited coverage rejects {name} {version}: {error}"
            );
            return Ok(());
        }
    }
    Err("probe dependency is not present in an audited non-root package".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context() -> Context {
        serde_json::from_value(json!({
            "producer":"application", "target":"x86_64-unknown-linux-gnu", "roots":["application"],
            "default_features":true, "features":[], "build_flags":["--release"],
            "build_command":"cargoBuildHook", "toolchain":{"rust":"pinned-toolchain"},
            "exceptions":"exceptions.json", "lock_file":"Cargo.lock", "evidence_documents":{}
        }))
        .unwrap()
    }

    fn lock(version: &str) -> String {
        format!(
            "version = 4\n\n[[package]]\nname = \"application\"\nversion = \"{version}\"\ndependencies = [\n \"dependency\",\n]\n\n[[package]]\nname = \"dependency\"\nversion = \"2\"\nsource = \"registry+https://example.org\"\nchecksum = \"pinned\"\n"
        )
    }

    fn binaries(path: PathBuf, version: &str) -> Vec<Binary> {
        vec![Binary {
            path,
            packages: vec![
                json!({"name":"application","version":version,"root":true,"source":"local"}),
                json!({"name":"dependency","version":"2","source":"registry"}),
            ],
        }]
    }

    #[test]
    fn code_and_local_version_changes_keep_exceptions_but_dependency_changes_fail_closed() {
        let temp = std::env::temp_dir().join(format!("eon-audit-context-{}", std::process::id()));
        fs::create_dir(&temp).unwrap();
        let binary = temp.join("application");
        fs::write(&binary, "old binary bytes").unwrap();
        let old_hash = digest(&binary).unwrap();
        let shipped = BTreeSet::from([("application".into(), "1".into())]);
        let first = snapshot(
            context(),
            &lock("1"),
            shipped,
            &[],
            binaries(binary.clone(), "1"),
            &temp.join("first"),
        )
        .unwrap();
        let exceptions = vec![AuditException {
            root: "application".into(),
            package: ("dependency".into(), "2".into()),
            graph_sha256: first.graph_sha256.clone(),
            reason: "proven unselected dependency".into(),
            evidence: "evidence.md#application".into(),
        }];
        fs::write(&binary, "changed application code and binary bytes").unwrap();
        assert_ne!(old_hash, digest(&binary).unwrap());
        let shipped = BTreeSet::from([("application".into(), "0.2.0".into())]);
        let changed = snapshot(
            context(),
            &lock("0.2.0"),
            shipped.clone(),
            &[],
            binaries(binary.clone(), "0.2.0"),
            &temp.join("changed"),
        )
        .unwrap();
        assert_eq!(first.graph_sha256, changed.graph_sha256);
        check_binary_crates(
            &binary,
            &changed.binaries[0].packages,
            &shipped,
            &exceptions,
            &mut BTreeSet::new(),
            &changed.graph_sha256,
        )
        .unwrap();

        // Even a newly covered dependency invalidates the previous over-report evidence.
        let mut gained = changed.binaries.clone();
        gained[0]
            .packages
            .push(json!({"name":"new-dependency","version":"3","source":"registry"}));
        let mut shipped = shipped;
        shipped.insert(("new-dependency".into(), "3".into()));
        let new_lock = format!(
            "{}\n\n[[package]]\nname = \"new-dependency\"\nversion = \"3\"\nsource = \"registry+https://example.org\"\n",
            lock("0.2.0")
        );
        let gained = snapshot(
            context(),
            &new_lock,
            shipped.clone(),
            &[],
            gained,
            &temp.join("gained"),
        )
        .unwrap();
        assert_ne!(first.graph_sha256, gained.graph_sha256);
        let error = check_binary_crates(
            &binary,
            &gained.binaries[0].packages,
            &shipped,
            &exceptions,
            &mut BTreeSet::new(),
            &gained.graph_sha256,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match dependency context")
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn context_covers_producer_flags_target_lock_and_resolved_features() {
        let context = context();
        let shipped = BTreeSet::from([("application".into(), "1".into())]);
        let binaries = binaries("/old/application".into(), "1");
        let original = graph(&context, &lock("1"), &shipped, &[], &binaries).unwrap();
        for field in [
            "producer",
            "target",
            "default_features",
            "features",
            "build_flags",
            "build_command",
            "toolchain",
        ] {
            let mut value = serde_json::to_value(&context).unwrap();
            value[field] = match field {
                "default_features" => json!(false),
                "features" | "build_flags" => json!(["changed"]),
                "toolchain" => json!({"rust":"other"}),
                _ => json!("changed"),
            };
            assert_ne!(
                original,
                graph(
                    &serde_json::from_value(value).unwrap(),
                    &lock("1"),
                    &shipped,
                    &[],
                    &binaries
                )
                .unwrap(),
                "{field}"
            );
        }
        let native = native_packages("dependency v2 features=changed").unwrap();
        assert_ne!(
            original,
            graph(&context, &lock("1"), &shipped, &native, &binaries).unwrap()
        );
        assert_ne!(
            original,
            graph(
                &context,
                &lock("1").replace("pinned", "other"),
                &shipped,
                &[],
                &binaries
            )
            .unwrap()
        );
        assert_eq!(
            lock_graph(&lock("1")).unwrap().0,
            lock_graph(&lock("0.2.0")).unwrap().0
        );
        assert!(lock_graph("not a lockfile").is_err());
        let refs = format!(
            "{}\n\n[[package]]\nname = \"local-dependent\"\nversion = \"1\"\ndependencies = [\n \"application 1\",\n]\n",
            lock("1")
        );
        assert_eq!(
            lock_graph(&refs).unwrap().0,
            lock_graph(
                &refs.replace("application 1", "application 0.2.0").replacen(
                    "version = \"1\"",
                    "version = \"0.2.0\"",
                    1
                )
            )
            .unwrap()
            .0
        );
    }

    #[test]
    fn local_proc_macro_origins_ignore_sandbox_and_store_source_paths() {
        let context = context();
        let lock = format!(
            "{}\n\n[[package]]\nname = \"local-macro\"\nversion = \"1\"\n",
            lock("1")
        );
        let shipped = BTreeSet::from([("application".into(), "1".into())]);
        let binaries = binaries("binary".into(), "1");
        let cached = native_packages("local-macro v1 (proc-macro) (/nix/store/hash-source/crates/local-macro) features=derive").unwrap();
        let sandbox = native_packages(
            "local-macro v1 (proc-macro) (/build/source/crates/local-macro) features=derive",
        )
        .unwrap();
        let expected = graph(&context, &lock, &shipped, &cached, &binaries).unwrap();
        assert_eq!(
            expected,
            graph(&context, &lock, &shipped, &sandbox, &binaries).unwrap()
        );
        let changed = native_packages(
            "local-macro v1 (proc-macro) (/build/source/crates/local-macro) features=other",
        )
        .unwrap();
        assert_ne!(
            expected,
            graph(&context, &lock, &shipped, &changed, &binaries).unwrap()
        );
    }

    #[test]
    fn renewal_refuses_changed_gaps_variants_and_missing_evidence() {
        let fixture = crate::tests::Fixture::new();
        let mut context = context();
        context
            .evidence_documents
            .insert("evidence.md".into(), fixture.0.join("evidence.md"));
        fs::write(
            fixture.0.join("evidence.md"),
            "## application\nObserved dependency paths.\n",
        )
        .unwrap();
        let snapshot = snapshot(
            context,
            &lock("1"),
            BTreeSet::from([("application".into(), "1".into())]),
            &[],
            binaries("binary".into(), "1"),
            &fixture.0.join("audit"),
        )
        .unwrap();
        let exceptions = fixture.0.join("exceptions.json");
        fs::write(
            &exceptions,
            serde_json::to_vec(&json!({"application":[{
                "root":"application", "graph_sha256":"old", "packages":[["dependency","2"]],
                "reason":"proven unselected", "evidence":"evidence.md#application"
            }]}))
            .unwrap(),
        )
        .unwrap();
        let mut input = fixture.input();
        input["entries"][0]["cargo_reports"] = json!([fixture.0.join("report.json")]);
        let full = fixture.0.join("full.json");
        fs::write(&full, serde_json::to_vec(&input).unwrap()).unwrap();
        let proposal = fixture.0.join("proposal.json");
        propose(&exceptions, &full, &full, &proposal).unwrap();
        let proposed: serde_json::Value =
            serde_json::from_slice(&fs::read(&proposal).unwrap()).unwrap();
        assert_eq!(
            proposed["application"][0]["graph_sha256"],
            snapshot.graph_sha256
        );
        assert!(propose(&exceptions, &full, &full, &proposal).is_err());

        let mut changed = serde_json::to_value(&snapshot).unwrap();
        changed["graph_sha256"] = json!("different");
        fs::create_dir_all(fixture.0.join("fallback/audit")).unwrap();
        fs::write(
            fixture.0.join("fallback/audit/observation.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        input["entries"][0]["cargo_reports"] = json!([fixture.0.join("fallback/report.json")]);
        let fallback = fixture.0.join("fallback.json");
        fs::write(&fallback, serde_json::to_vec(&input).unwrap()).unwrap();
        let rejected = fixture.0.join("rejected.json");
        assert!(
            propose(&exceptions, &full, &fallback, &rejected)
                .unwrap_err()
                .to_string()
                .contains("different audit contexts")
        );
        input["entries"] = json!([]);
        fs::write(&fallback, serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(
            propose(&exceptions, &full, &fallback, &rejected)
                .unwrap_err()
                .to_string()
                .contains("different Rust producer sets")
        );

        changed["graph_sha256"] = json!(&snapshot.graph_sha256);
        changed["missing"]["application"] = json!([["new", "3"]]);
        fs::write(
            fixture.0.join("audit/observation.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(
            propose(&exceptions, &full, &full, &rejected)
                .unwrap_err()
                .to_string()
                .contains("changed exception identities")
        );
        fs::write(
            fixture.0.join("audit/observation.json"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        fs::write(fixture.0.join("evidence.md"), "## renamed\n").unwrap();
        assert!(
            propose(&exceptions, &full, &full, &rejected)
                .unwrap_err()
                .to_string()
                .contains("missing audit evidence anchor")
        );
        assert!(!rejected.exists());
    }
}
