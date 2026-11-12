//! Source delivery over the obligations in the exact binary notice inventory.
use super::*;
use std::path::Component;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub filename: String,
    pub releases_page: String,
    pub support: BTreeMap<String, PathBuf>,
    pub instructions: PathBuf,
    pub runtime_exception_basis: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub sources: Vec<PathBuf>,
    pub patches: Vec<PathBuf>,
    pub build: PathBuf,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct Delivery {
    licenses: BTreeSet<String>,
    identity: String,
    producer: Producer,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Form {
    DirectoryTar,
    Archive,
    File,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Material {
    path: PathBuf,
    sha256: String,
    bytes: u64,
    form: Form,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    inventory_sha256: String,
    filename: String,
    releases_page: String,
    runtime_exception_basis: String,
    build_support: BTreeMap<String, PathBuf>,
    obligations: BTreeMap<String, Delivery>,
    materials: BTreeMap<PathBuf, Material>,
}

fn covered(license: &str) -> Result<bool> {
    match license {
        "MPL-2.0"
        | "GPL-2.0-or-later"
        | "GPL-3.0-only"
        | "GPL-3.0-or-later"
        | "LGPL-2.0-or-later"
        | "LGPL-2.1-only"
        | "LGPL-2.1-or-later"
        | "LGPL-3.0-or-later"
        | "GPL-3.0-or-later WITH GCC-exception-3.1"
        | "GPL-2.0-only WITH GCC-exception-2.0" => Ok(true),
        id if ["GPL-", "AGPL-", "LGPL-", "MPL-", "EPL-", "EUPL-", "CDDL-"]
            .iter()
            .any(|prefix| id.starts_with(prefix)) =>
        {
            Err(format!("unreviewed source license: {id}").into())
        }
        _ => Ok(false),
    }
}

fn licenses(ids: &[String]) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for id in ids {
        if covered(id)? {
            result.insert(id.clone());
        }
    }
    Ok(result)
}

fn requirements(notices: &Path, report: &Report) -> Result<BTreeMap<String, Delivery>> {
    if report.schema != 1 {
        return Err("unsupported notice inventory schema".into());
    }
    let mut required = BTreeMap::new();
    for entry in &report.entries {
        let ids = licenses(&entry.licenses)?;
        if !ids.is_empty() {
            required.insert(
                entry.id.clone(),
                Delivery {
                    licenses: ids,
                    identity: entry.source.clone(),
                    producer: entry
                        .source_material
                        .clone()
                        .ok_or_else(|| format!("{}: missing producer source", entry.id))?,
                },
            );
        }
        for file in report.files.iter().filter(|file| {
            file.owner == entry.id && file.path.contains("-cargo-") && file.path.ends_with(".json")
        }) {
            relative(Path::new(&file.path))?;
            if digest(&notices.join(&file.path))? != file.sha256 {
                return Err("Cargo notice report changed".into());
            }
            let cargo: serde_json::Value =
                serde_json::from_slice(&fs::read(notices.join(&file.path))?)?;
            let mut crate_licenses: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
            for license in cargo["licenses"]
                .as_array()
                .ok_or("missing Cargo licenses")?
            {
                let id = license["id"].as_str().ok_or("missing Cargo license ID")?;
                if covered(id)? {
                    for usage in license["used_by"]
                        .as_array()
                        .ok_or("missing Cargo license users")?
                    {
                        if (id.starts_with("GPL-") || id.starts_with("LGPL-"))
                            && !matches!(
                                id,
                                "GPL-3.0-or-later WITH GCC-exception-3.1"
                                    | "GPL-2.0-only WITH GCC-exception-2.0"
                            )
                        {
                            let (name, version) = cargo_identity(&usage["crate"])?;
                            return Err(format!("statically linked Cargo crate {name} {version} uses {id}; source/relink terms require a shipment decision").into());
                        }
                        crate_licenses
                            .entry(cargo_identity(&usage["crate"])?)
                            .or_default()
                            .insert(id.into());
                    }
                }
            }
            for item in cargo["crates"].as_array().ok_or("missing Cargo crates")? {
                let package = &item["package"];
                let (name, version) = cargo_identity(package)?;
                let Some(ids) = crate_licenses.remove(&(name.clone(), version.clone())) else {
                    continue;
                };
                let mut producer = entry
                    .source_material
                    .clone()
                    .ok_or_else(|| format!("{}: missing Cargo producer", entry.id))?;
                let identity = if package["source"].is_null() {
                    format!("{name}@{version} in {}", entry.source)
                } else {
                    let source = package["source"].as_str().ok_or("missing crate source")?;
                    if source != "registry+https://github.com/rust-lang/crates.io-index" {
                        return Err(format!("unsupported Cargo source form: {source}").into());
                    }
                    let checksum = item["source_checksum"]
                        .as_str()
                        .ok_or("missing crate checksum")?;
                    valid_hash(checksum)?;
                    producer.sources = vec![
                        item["source_archive"]
                            .as_str()
                            .ok_or("missing retained crate source")?
                            .into(),
                    ];
                    format!(
                        "https://static.crates.io/crates/{name}/{name}-{version}.crate sha256:{checksum}"
                    )
                };
                required.insert(
                    format!("{} / {name} {version}", entry.id),
                    Delivery {
                        licenses: ids,
                        identity,
                        producer,
                    },
                );
            }
            if !crate_licenses.is_empty() {
                return Err("copyleft Cargo license lacks its crate source identity".into());
            }
        }
    }
    let inventory: BTreeMap<_, _> = report
        .source_obligations
        .iter()
        .map(|(id, terms)| (id.clone(), terms.keys().cloned().collect::<BTreeSet<_>>()))
        .collect();
    let selected: BTreeMap<_, _> = required
        .iter()
        .map(|(id, delivery)| (id.clone(), delivery.licenses.clone()))
        .collect();
    if inventory != selected {
        return Err("source obligations disagree with inventoried licenses".into());
    }
    for (id, delivery) in &required {
        if delivery.identity.trim().is_empty()
            || delivery.producer.sources.is_empty()
            || delivery.producer.build.as_os_str().is_empty()
        {
            return Err(format!("{id}: incomplete source/build material").into());
        }
    }
    Ok(required)
}

fn valid_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid source checksum".into());
    }
    Ok(())
}

fn relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(format!("source path escapes companion: {}", path.display()).into());
    }
    Ok(())
}

fn directory_files(root: &Path, path: &Path) -> Result<usize> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() {
        let target = fs::read_link(path)?;
        if target.is_absolute() {
            return Err(format!("absolute source symlink: {}", path.display()).into());
        }
        let mut depth = path.strip_prefix(root)?.components().count() - 1;
        for component in target.components() {
            match component {
                Component::ParentDir => {
                    depth = depth.checked_sub(1).ok_or("source symlink escapes tree")?
                }
                Component::Normal(_) => depth += 1,
                Component::CurDir => {}
                _ => return Err("source symlink escapes tree".into()),
            }
        }
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(usize::from(metadata.len() > 0));
    }
    if !metadata.is_dir() {
        return Err("unsupported source file type".into());
    }
    let mut count = 0;
    for entry in fs::read_dir(path)? {
        count += directory_files(root, &entry?.path())?;
    }
    Ok(count)
}

fn archive_directory(root: &Path, target: &Path) -> Result<()> {
    if directory_files(root, root)? == 0 {
        return Err(format!("empty source tree: {}", root.display()).into());
    }
    let result = Command::new("tar")
        .args([
            "--sort=name",
            "--mtime=@1",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-cf",
        ])
        .arg(target)
        .arg("-C")
        .arg(root)
        .arg(".")
        .status()?;
    if !result.success() {
        return Err(format!("cannot archive source tree: {}", root.display()).into());
    }
    Ok(())
}

pub fn retain_crate(item: &mut serde_json::Value, root: &Path, output: &Path) -> Result<()> {
    let package = &item["package"];
    if package["source"] != "registry+https://github.com/rust-lang/crates.io-index" {
        return Err("unsupported copyleft Cargo source form".into());
    }
    let (name, version) = cargo_identity(package)?;
    let filename = format!("{name}-{version}.tar");
    relative(Path::new(&filename))?;
    let checksums: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".cargo-checksum.json"))?)?;
    let checksum = checksums["package"]
        .as_str()
        .ok_or("vendored source lacks package checksum")?;
    valid_hash(checksum)?;
    let directory = output.join("sources");
    fs::create_dir_all(&directory)?;
    let target = directory.join(filename);
    archive_directory(root, &target)?;
    item["source_archive"] = serde_json::to_value(target)?;
    item["source_checksum"] = checksum.into();
    Ok(())
}

fn material_paths<'a>(
    release: &'a Release,
    obligations: &'a BTreeMap<String, Delivery>,
) -> BTreeMap<PathBuf, bool> {
    let mut paths = BTreeMap::new();
    for path in release
        .support
        .values()
        .chain(std::iter::once(&release.instructions))
    {
        paths.insert(path.clone(), false);
    }
    for delivery in obligations.values() {
        for path in delivery
            .producer
            .patches
            .iter()
            .chain(std::iter::once(&delivery.producer.build))
        {
            paths.entry(path.clone()).or_insert(false);
        }
        for path in &delivery.producer.sources {
            paths.insert(path.clone(), true);
        }
    }
    paths
}

pub fn generate(notices: &Path, output: &Path) -> Result<()> {
    let report: Report = serde_json::from_slice(&fs::read(notices.join("inventory.json"))?)?;
    let obligations = requirements(notices, &report)?;
    fs::create_dir(output)?;
    fs::create_dir(output.join("materials"))?;
    let mut materials = BTreeMap::new();
    let mut content_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (index, (origin, is_source)) in material_paths(&report.source_release, &obligations)
        .into_iter()
        .enumerate()
    {
        let resolved = fs::canonicalize(&origin)?;
        let name = origin
            .file_name()
            .ok_or("source material has no name")?
            .to_string_lossy();
        let directory = resolved.is_dir();
        let mut path = PathBuf::from(format!(
            "materials/{index}-{name}{}",
            if directory { ".tar" } else { "" }
        ));
        relative(&path)?;
        let target = output.join(&path);
        let form = if directory {
            archive_directory(&resolved, &target)?;
            Form::DirectoryTar
        } else {
            if !resolved.is_file() || fs::metadata(&resolved)?.len() == 0 {
                return Err(
                    format!("missing or empty source material: {}", origin.display()).into(),
                );
            }
            let archive = [
                ".tar", ".tar.gz", ".tar.xz", ".tar.lz", ".tar.bz2", ".tar.zst", ".tgz", ".crate",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix));
            if is_source && !archive && !name.ends_with(".txt") {
                return Err(format!("unsupported source form: {}", origin.display()).into());
            }
            fs::copy(&resolved, &target)?;
            if archive { Form::Archive } else { Form::File }
        };
        let sha256 = digest(&target)?;
        let bytes = fs::metadata(&target)?.len();
        if let Some(existing) = content_paths.get(&sha256) {
            fs::remove_file(&target)?;
            path = existing.clone();
        } else {
            content_paths.insert(sha256.clone(), path.clone());
        }
        materials.insert(
            origin,
            Material {
                path,
                sha256,
                bytes,
                form,
            },
        );
    }
    let instructions = output.join("BUILD-AND-RELINK.md");
    fs::copy(&report.source_release.instructions, &instructions)?;
    let index = Index {
        schema: 1,
        inventory_sha256: digest(&notices.join("inventory.json"))?,
        filename: report.source_release.filename,
        releases_page: report.source_release.releases_page,
        runtime_exception_basis: report.source_release.runtime_exception_basis,
        build_support: report.source_release.support,
        obligations,
        materials,
    };
    fs::write(
        output.join("source-index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;
    fs::write(
        output.join("README.txt"),
        format!(
            "Eon source companion: {}\n\nAttached to the same GitHub release as the binary archive. Releases: {}\nSee source-index.json for every inventoried source obligation, its exact source identity, source/patch/build origins, and their local materials paths and SHA-256 hashes. Directory sources are deterministic tar files. Archive sources retain their upstream bytes. BUILD-AND-RELINK.md describes rebuilding and replacement.\n",
            index.filename, index.releases_page
        ),
    )?;
    check(notices, output)
}

fn references_metadata(value: &serde_json::Value, origin: &str) -> bool {
    match value {
        serde_json::Value::String(text) => {
            text.contains(origin) || text.contains("-source-build.json")
        }
        serde_json::Value::Array(items) => {
            items.iter().any(|item| references_metadata(item, origin))
        }
        serde_json::Value::Object(fields) => fields
            .values()
            .any(|item| references_metadata(item, origin)),
        _ => false,
    }
}

pub fn check(notices: &Path, output: &Path) -> Result<()> {
    let report: Report = serde_json::from_slice(&fs::read(notices.join("inventory.json"))?)?;
    let index: Index = serde_json::from_slice(&fs::read(output.join("source-index.json"))?)?;
    if index.schema != 1
        || index.inventory_sha256 != digest(&notices.join("inventory.json"))?
        || index.filename != report.source_release.filename
        || index.releases_page != report.source_release.releases_page
        || index.runtime_exception_basis != report.source_release.runtime_exception_basis
        || index.runtime_exception_basis.trim().is_empty()
        || index.build_support != report.source_release.support
    {
        return Err("source index does not match binary notice inventory".into());
    }
    if index.obligations != requirements(notices, &report)? {
        return Err("source index lacks or changes an inventoried obligation".into());
    }
    let expected = material_paths(&report.source_release, &index.obligations);
    if expected.keys().ne(index.materials.keys()) {
        return Err(
            "source material list differs from required sources, patches and build support".into(),
        );
    }
    for material in index.materials.values() {
        relative(&material.path)?;
        let path = output.join(&material.path);
        if fs::symlink_metadata(&path)?.is_symlink()
            || !fs::canonicalize(&path)?.starts_with(fs::canonicalize(output)?)
            || material.bytes == 0
            || fs::metadata(&path)?.len() != material.bytes
            || digest(&path)? != material.sha256
        {
            return Err(format!(
                "source material is missing or changed: {}",
                material.path.display()
            )
            .into());
        }
    }
    for origin in index
        .obligations
        .values()
        .map(|delivery| &delivery.producer.build)
        .collect::<BTreeSet<_>>()
    {
        if origin
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let material = index
                .materials
                .get(origin)
                .ok_or("missing build metadata")?;
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(output.join(&material.path))?)?;
            if references_metadata(&value, &origin.to_string_lossy()) {
                return Err(format!(
                    "producer output references source metadata: {}",
                    origin.display()
                )
                .into());
            }
        }
    }
    let instructions = index
        .materials
        .get(&report.source_release.instructions)
        .ok_or("missing instructions")?;
    if digest(&output.join("BUILD-AND-RELINK.md"))? != instructions.sha256 {
        return Err("build/relink instructions are changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::Fixture;
    use serde_json::json;

    fn fixture() -> Fixture {
        let fixture = Fixture::new();
        let root = &fixture.0;
        fs::create_dir(root.join("source")).unwrap();
        fs::write(root.join("source/main.c"), "int main(void) { return 0; }\n").unwrap();
        fs::create_dir(root.join("source-copy")).unwrap();
        fs::copy(root.join("source/main.c"), root.join("source-copy/main.c")).unwrap();
        fs::write(root.join("patch"), "a selected upstream patch\n").unwrap();
        fs::write(
            root.join("build.json"),
            "{\"configureFlags\":[\"--shared\"]}\n",
        )
        .unwrap();
        fs::write(
            root.join("BUILD.md"),
            "Rebuild with the supplied recipe; replace the shared library.\n",
        )
        .unwrap();
        let mut input = fixture.input();
        input["accepted"] = json!(["MIT", "GPL-3.0-or-later", "MPL-2.0", "LGPL-2.1-only"]);
        input["entries"][0]["licenses"] = json!(["GPL-3.0-or-later", "MPL-2.0", "LGPL-2.1-only"]);
        input["entries"][0]["source_material"] = json!({
            "sources":[root.join("source"),root.join("source-copy")],"patches":[root.join("patch")],"build":root.join("build.json")
        });
        fixture.generate(input);
        fixture
    }

    #[test]
    fn exact_sources_patches_and_inventory_are_required() {
        let fixture = fixture();
        let notices = fixture.0.join("notices");
        let output = fixture.0.join("companion");
        generate(&notices, &output).unwrap();
        check(&notices, &output).unwrap();
        let index: Index =
            serde_json::from_slice(&fs::read(output.join("source-index.json")).unwrap()).unwrap();
        assert_eq!(index.obligations["tool"].licenses.len(), 3);
        assert_eq!(index.materials.len(), 5);
        assert_eq!(
            index
                .materials
                .values()
                .map(|material| &material.path)
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
        let patch = &index.materials[&fixture.0.join("patch")];
        fs::write(output.join(&patch.path), "changed").unwrap();
        assert!(check(&notices, &output).is_err());
        fs::copy(fixture.0.join("patch"), output.join(&patch.path)).unwrap();
        fs::remove_file(output.join(&patch.path)).unwrap();
        assert!(check(&notices, &output).is_err());
        fs::copy(fixture.0.join("patch"), output.join(&patch.path)).unwrap();
        let inventory = notices.join("inventory.json");
        let mut bytes = fs::read(&inventory).unwrap();
        bytes.push(b'\n');
        fs::write(inventory, bytes).unwrap();
        assert!(check(&notices, &output).is_err());
    }

    #[test]
    fn incomplete_mapping_unknown_license_and_path_escape_fail_closed() {
        let fixture = fixture();
        let notices = fixture.0.join("notices");
        let output = fixture.0.join("companion");
        generate(&notices, &output).unwrap();
        let path = output.join("source-index.json");
        let original = fs::read(&path).unwrap();
        let mut index: Index = serde_json::from_slice(&original).unwrap();
        index.obligations.clear();
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(check(&notices, &output).is_err());
        let mut index: Index = serde_json::from_slice(&original).unwrap();
        index.materials.values_mut().next().unwrap().path = "../patch".into();
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(check(&notices, &output).is_err());
        let mut report: Report =
            serde_json::from_slice(&fs::read(notices.join("inventory.json")).unwrap()).unwrap();
        report.entries[0].licenses = vec!["MPL-9.0".into()];
        assert!(requirements(&notices, &report).is_err());
        assert!(covered("EPL-2.0").is_err());
    }

    #[test]
    fn producer_output_references_cannot_point_into_build_metadata() {
        let fixture = fixture();
        let notices = fixture.0.join("notices");
        let build = fixture.0.join("build.json");
        for (index, prefix) in ["@output:out@".to_owned(), "/producer/output".to_owned()]
            .into_iter()
            .enumerate()
        {
            fs::write(&build, serde_json::to_vec(&json!({"configuration": {
                "NIX_CFLAGS_COMPILE":[format!("-DDEFAULT_LOADABLE_BUILTINS_PATH=\"{prefix}/lib/bash:.\"")]
            }})).unwrap()).unwrap();
            generate(&notices, &fixture.0.join(format!("valid-{index}"))).unwrap();
        }
        for (index, bad) in [
            build.display().to_string(),
            "/nix/store/hash-bash-source-build.json".into(),
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&build, serde_json::to_vec(&json!({"configuration": {
                "nested":{"flags":[format!("-DDEFAULT_LOADABLE_BUILTINS_PATH=\"{bad}/lib/bash:.\"")]}
            }})).unwrap()).unwrap();
            let error = generate(&notices, &fixture.0.join(format!("bad-{index}"))).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("producer output references source metadata")
            );
        }
    }

    #[test]
    fn retained_registry_sources_keep_exact_identity_and_reject_escaping_links() {
        let fixture = Fixture::new();
        let root = fixture.0.join("crate");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("lib.rs"), "pub fn covered_code() {}\n").unwrap();
        fs::write(
            root.join(".cargo-checksum.json"),
            format!("{{\"package\":\"{}\"}}", "a".repeat(64)),
        )
        .unwrap();
        let mut item = json!({"package":{"name":"covered","version":"1.0.0",
            "source":"registry+https://github.com/rust-lang/crates.io-index"}});
        retain_crate(&mut item, &root, &fixture.0).unwrap();
        let archive = item["source_archive"].as_str().unwrap();
        assert_eq!(item["source_checksum"], "a".repeat(64));
        let listing = Command::new("tar").args(["-tf", archive]).output().unwrap();
        assert!(listing.status.success());
        assert!(
            String::from_utf8(listing.stdout)
                .unwrap()
                .contains("./lib.rs")
        );
        std::os::unix::fs::symlink("../../outside", root.join("escape")).unwrap();
        assert!(retain_crate(&mut item, &root, &fixture.0).is_err());
    }

    #[test]
    fn named_build_support_roles_survive_delivery() {
        let fixture = fixture();
        let notices = fixture.0.join("notices");
        let inventory = notices.join("inventory.json");
        let mut report: serde_json::Value =
            serde_json::from_slice(&fs::read(&inventory).unwrap()).unwrap();
        let mut support = serde_json::Map::new();
        for (index, role) in ["nixpkgs", "eon", "helix"].into_iter().enumerate() {
            let root = fixture.0.join(format!("opaque-tree-{index}"));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("contents"), role).unwrap();
            support.insert(role.into(), json!(root));
        }
        report["source_release"]["support"] = support.clone().into();
        fs::write(&inventory, serde_json::to_vec(&report).unwrap()).unwrap();
        let output = fixture.0.join("companion");
        generate(&notices, &output).unwrap();
        let path = output.join("source-index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(index["build_support"], serde_json::Value::Object(support));
        for origin in index["build_support"].as_object().unwrap().values() {
            assert!(
                output
                    .join(
                        index["materials"][origin.as_str().unwrap()]["path"]
                            .as_str()
                            .unwrap()
                    )
                    .is_file()
            );
        }
        index["build_support"]
            .as_object_mut()
            .unwrap()
            .remove("eon");
        fs::write(path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(check(&notices, &output).is_err());
    }

    #[test]
    fn cargo_copyleft_requires_a_supported_static_linking_basis() {
        for (license, allowed) in [
            ("MPL-2.0", true),
            ("GPL-3.0-or-later", false),
            ("LGPL-2.1-only", false),
            ("GPL-3.0-or-later WITH GCC-exception-3.1", true),
            ("GPL-2.0-only WITH GCC-exception-2.0", true),
        ] {
            let fixture = fixture();
            let notices = fixture.0.join("notices");
            let package = json!({"name":"covered-rust","version":"1.0.0","source":null});
            let cargo = json!({"crates":[{"package":package}],"licenses":[
                {"id":license,"used_by":[{"crate":package}]}
            ]});
            let path = "texts/static-cargo-0.json";
            fs::write(notices.join(path), serde_json::to_vec(&cargo).unwrap()).unwrap();
            let inventory = notices.join("inventory.json");
            let mut report: Report =
                serde_json::from_slice(&fs::read(&inventory).unwrap()).unwrap();
            report.files.push(File {
                owner: "tool".into(),
                source: "fixture Cargo report".into(),
                path: path.into(),
                sha256: digest(&notices.join(path)).unwrap(),
            });
            report.source_obligations.insert(
                "tool / covered-rust 1.0.0".into(),
                BTreeMap::from([(license.into(), source_obligation(license).unwrap().into())]),
            );
            fs::write(inventory, serde_json::to_vec(&report).unwrap()).unwrap();
            let result = generate(&notices, &fixture.0.join("companion"));
            if allowed {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert!(error.to_string().contains("statically linked Cargo crate"));
                assert!(error.to_string().contains("require a shipment decision"));
            }
        }
    }
}
