//! Build-time notice assembly and publication gate over the actual relinked tree.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    schema: u32,
    accepted: BTreeSet<String>,
    required_components: BTreeSet<String>,
    omitted_grammars: BTreeSet<String>,
    #[serde(default)]
    binary_inputs: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    native_exclusions: BTreeMap<String, NativeExclusion>,
    entries: Vec<Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    version: String,
    source: String,
    licenses: Vec<String>,
    #[serde(default)]
    components: BTreeSet<String>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    notice_files: Vec<PathBuf>,
    #[serde(default)]
    source_roots: Vec<PathBuf>,
    #[serde(default)]
    notice_hashes: BTreeMap<PathBuf, String>,
    #[serde(default)]
    checks: Vec<PathBuf>,
    #[serde(default)]
    unresolved: Vec<String>,
    #[serde(default)]
    forbidden_embedded_files: Vec<PathBuf>,
    #[serde(default)]
    output_name: Option<String>,
    #[serde(default)]
    cargo_reports: Vec<PathBuf>,
    #[serde(default)]
    cargo_source_licenses: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    native_bindings: Vec<NativeBinding>,
    #[serde(default)]
    upstream_distribution: Option<String>,
    #[serde(default)]
    embedded: Vec<String>,
    #[serde(default)]
    grammar: Option<String>,
    #[serde(default)]
    grant: Option<Grant>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    rights_holder: String,
    record: PathBuf,
    sha256: String,
    public_binary_redistribution: bool,
    modified_regular_font: bool,
    modified_bold_font: bool,
    bitmap_glyphs: bool,
    recipients_rights: String,
    attribution: String,
    revocability: String,
    assets: BTreeMap<PathBuf, String>,
}

#[derive(Deserialize, Serialize)]
struct File {
    owner: String,
    source: String,
    path: String,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
struct Binary {
    owner: String,
    sha256: String,
    embedded: Vec<String>,
    native_dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct NativeExclusion {
    versions: BTreeSet<String>,
    reason: String,
    #[serde(default)]
    forbidden_features: BTreeSet<String>,
}

#[derive(Deserialize, Serialize)]
struct NativeBinding {
    name: String,
    version: String,
    notice: String,
    #[serde(default)]
    features: BTreeSet<String>,
    #[serde(default)]
    forbidden_features: BTreeSet<String>,
    #[serde(default)]
    build_package: Option<(String, String)>,
}

#[derive(Deserialize, Serialize)]
struct NativePackage {
    name: String,
    version: String,
    features: BTreeSet<String>,
    origin: String,
}

#[derive(Deserialize, Serialize)]
struct NativeBinary {
    root: (String, String),
    packages: BTreeSet<(String, String)>,
}

#[derive(Deserialize, Serialize)]
struct Report {
    schema: u32,
    entries: Vec<Entry>,
    files: Vec<File>,
    binaries: BTreeMap<String, Binary>,
    notice_obligations: BTreeMap<String, String>,
    source_obligations: BTreeMap<String, BTreeMap<String, String>>,
    notice_gaps: Vec<String>,
    findings: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("archive-notices: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [command, input, tree, output] if command == "generate" => {
            generate(Path::new(input), Path::new(tree), Path::new(output))?;
        }
        [command, output] if command == "check" => check(Path::new(output))?,
        [command, build, modules, licenses, go_version] if command == "check-go" => check_go(
            &fs::read(Path::new(build))?,
            &fs::read_to_string(Path::new(modules))?,
            &fs::read_to_string(Path::new(licenses))?,
            go_version.to_str().ok_or("invalid Go version")?,
        )?,
        [
            command,
            report,
            output,
            binaries,
            tree,
            supplements,
            native_tree,
        ] if command == "cargo-texts" => {
            cargo_texts(
                Path::new(report),
                Path::new(output),
                Path::new(binaries),
                Path::new(tree),
                Path::new(supplements),
                Path::new(native_tree),
            )?;
        }
        _ => {
            return Err(
                "usage: archive-notices generate INPUT.json RELINKED-TREE OUTPUT | check OUTPUT"
                    .into(),
            );
        }
    }
    Ok(())
}

fn check_go(build: &[u8], modules: &str, licenses: &str, go_version: &str) -> Result<()> {
    let info: serde_json::Value = serde_json::from_slice(build)?;
    if info["GoVersion"].as_str() != Some(format!("go{go_version}").as_str()) {
        return Err("Go toolchain does not match binary".into());
    }
    let covered: BTreeSet<_> = licenses
        .lines()
        .filter_map(|row| row.split_once(',').map(|(name, _)| name))
        .collect();
    for dependency in info["Deps"]
        .as_array()
        .ok_or("Go build information has no dependencies")?
    {
        let path = dependency["Path"].as_str().ok_or("Go module has no path")?;
        let version = dependency["Version"]
            .as_str()
            .ok_or("Go module has no version")?;
        let mut header = format!("# {path} {version}");
        if let Some(replacement) = dependency["Replace"].as_object() {
            let replacement_path = replacement["Path"]
                .as_str()
                .ok_or("Go replacement has no path")?;
            header.push_str(&format!(" => {replacement_path}"));
            if let Some(version) = replacement["Version"]
                .as_str()
                .filter(|version| !version.is_empty())
            {
                header.push(' ');
                header.push_str(version);
            }
        }
        if !modules.lines().any(|line| line == header) {
            return Err(format!(
                "Go binary dependency is absent from pinned vendor modules: {header}"
            )
            .into());
        }
        let replacement_path = dependency["Replace"]["Path"].as_str();
        if !covered.iter().any(|name| {
            std::iter::once(path).chain(replacement_path).any(|path| {
                *name == path
                    || name
                        .strip_prefix(path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        }) {
            return Err(format!("Go module lacks a license report entry: {path}@{version}").into());
        }
    }
    Ok(())
}

fn cargo_identity(package: &serde_json::Value) -> Result<(String, String)> {
    Ok((
        package["name"]
            .as_str()
            .ok_or("missing Cargo name")?
            .to_owned(),
        package["version"]
            .as_str()
            .ok_or("missing Cargo version")?
            .to_owned(),
    ))
}

fn audit_packages(binary: &Path) -> Result<Vec<serde_json::Value>> {
    let audit = Command::new("rust-audit-info").arg(binary).output()?;
    if !audit.status.success() {
        return Err(format!("missing Rust dependency evidence: {}", binary.display()).into());
    }
    let report: serde_json::Value = serde_json::from_slice(&audit.stdout)?;
    Ok(report["packages"]
        .as_array()
        .ok_or("empty binary Cargo evidence")?
        .clone())
}

fn binary_crates(binaries: &Path) -> Result<Vec<NativeBinary>> {
    let mut files = Vec::new();
    tree_files(binaries, &mut files)?;
    let mut crates = Vec::new();
    for file in files {
        if !is_elf(&file)? {
            continue;
        }
        let packages = audit_packages(&file)?;
        let identities = packages
            .iter()
            .map(cargo_identity)
            .collect::<Result<BTreeSet<_>>>()?;
        for package in &packages {
            if package["root"].as_bool() == Some(true) {
                crates.push(NativeBinary {
                    root: cargo_identity(package)?,
                    packages: identities.clone(),
                });
            }
        }
    }
    if crates.is_empty() {
        return Err("no Rust package evidence in shipped binaries".into());
    }
    Ok(crates)
}

fn native_packages(tree: &str) -> Result<Vec<NativePackage>> {
    let mut packages: BTreeMap<(String, String), (BTreeSet<String>, String)> = BTreeMap::new();
    for line in tree.lines().filter(|line| !line.trim().is_empty()) {
        let (identity, features) = line
            .strip_suffix(" (*)")
            .unwrap_or(line)
            .split_once(" features=")
            .ok_or("native Cargo tree lacks feature evidence")?;
        let mut fields = identity.split_whitespace();
        let name = fields
            .next()
            .ok_or("native Cargo tree lacks package name")?;
        let version = fields
            .next()
            .and_then(|v| v.strip_prefix('v'))
            .ok_or("native Cargo tree lacks version")?;
        let entry = packages
            .entry((name.into(), version.into()))
            .or_insert_with(|| (BTreeSet::new(), identity.into()));
        entry.0.extend(
            features
                .split(',')
                .filter(|f| !f.is_empty())
                .map(str::to_owned),
        );
    }
    Ok(packages
        .into_iter()
        .map(|((name, version), (features, origin))| NativePackage {
            name,
            version,
            features,
            origin,
        })
        .collect())
}

#[derive(Deserialize)]
struct CargoNotice {
    version: String,
    revision: String,
    files: Vec<PathBuf>,
}

fn supplement_cargo_notice(
    package: &serde_json::Value,
    root: &Path,
    output: &Path,
    notice: &CargoNotice,
) -> Result<()> {
    let vcs: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".cargo_vcs_info.json"))?)?;
    if package["version"] != notice.version || vcs["git"]["sha1"] != notice.revision {
        return Err("supplementary Cargo notice does not match the pinned crate".into());
    }
    fs::create_dir_all(output)?;
    for (index, file) in notice.files.iter().enumerate() {
        fs::copy(file, output.join(format!("LICENSE-upstream-{index}")))?;
    }
    fs::write(
        output.join("NOTICE-provenance.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"crate": package, "revision": notice.revision, "files": notice.files}),
        )?,
    )?;
    Ok(())
}

fn cargo_texts(
    report_path: &Path,
    output: &Path,
    binaries: &Path,
    tree: &Path,
    supplements: &Path,
    native_tree: &Path,
) -> Result<()> {
    let supplements: BTreeMap<String, CargoNotice> =
        serde_json::from_slice(&fs::read(supplements)?)?;
    let mut shipped = BTreeSet::new();
    for line in fs::read_to_string(tree)?
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let mut fields = line.split_whitespace();
        let name = fields.next().ok_or("Cargo tree has an empty line")?;
        let version = fields
            .next()
            .and_then(|field| field.strip_prefix('v'))
            .ok_or("Cargo tree has no version")?;
        shipped.insert((name.to_owned(), version.to_owned()));
    }
    let binaries = binary_crates(binaries)?;
    let roots: BTreeSet<_> = binaries.iter().map(|binary| binary.root.clone()).collect();
    if !roots.is_subset(&shipped) {
        return Err(format!(
            "shipped Rust roots absent from Cargo tree: {:?}",
            roots.difference(&shipped).collect::<Vec<_>>()
        )
        .into());
    }
    let mut report: serde_json::Value = serde_json::from_slice(&fs::read(report_path)?)?;
    report["native_packages"] =
        serde_json::to_value(native_packages(&fs::read_to_string(native_tree)?)?)?;
    report["native_binaries"] = serde_json::to_value(binaries)?;
    let mut missing = shipped.clone();
    let crates = report["crates"]
        .as_array_mut()
        .ok_or("Cargo report has no crates")?;
    crates.retain(|entry| {
        cargo_identity(&entry["package"]).is_ok_and(|identity| {
            missing.remove(&identity);
            shipped.contains(&identity)
        })
    });
    if !missing.is_empty() {
        return Err(format!("binary crates lack Cargo license metadata: {missing:?}").into());
    }
    for (index, entry) in crates.iter_mut().enumerate() {
        let manifest = entry["package"]["manifest_path"]
            .as_str()
            .ok_or("Cargo crate has no manifest")?;
        let root = fs::canonicalize(
            Path::new(manifest)
                .parent()
                .ok_or("Cargo manifest has no parent")?,
        )?;
        if entry["package"]["license"].as_str() == Some("CC-PDDC") {
            let vcs: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join(".cargo_vcs_info.json"))?)?;
            entry["source_revision"] = vcs["git"]["sha1"].clone();
        }
        for file in notice_files(&root)? {
            let target = output
                .join(index.to_string())
                .join(file.strip_prefix(&root)?);
            fs::create_dir_all(target.parent().ok_or("notice has no parent")?)?;
            fs::copy(file, target)?;
        }
        if let Some(notice) = entry["package"]["name"]
            .as_str()
            .and_then(|name| supplements.get(name))
        {
            supplement_cargo_notice(
                &entry["package"],
                &root,
                &output.join(index.to_string()),
                notice,
            )?;
        }
    }
    let licenses = report["licenses"]
        .as_array_mut()
        .ok_or("Cargo report has no licenses")?;
    for license in licenses.iter_mut() {
        let used = license["used_by"]
            .as_array_mut()
            .ok_or("Cargo license has no crate coverage")?;
        used.retain(|usage| {
            cargo_identity(&usage["crate"]).is_ok_and(|identity| shipped.contains(&identity))
        });
    }
    licenses.retain(|license| {
        license["used_by"]
            .as_array()
            .is_some_and(|used| !used.is_empty())
    });
    let covered = licenses
        .iter()
        .flat_map(|license| license["used_by"].as_array().into_iter().flatten())
        .map(|usage| cargo_identity(&usage["crate"]))
        .collect::<Result<BTreeSet<_>>>()?;
    if covered != shipped {
        return Err(format!(
            "binary crates lack selected license terms: {:?}",
            shipped.difference(&covered).collect::<Vec<_>>()
        )
        .into());
    }
    fs::write(report_path, serde_json::to_vec(&report)?)?;
    Ok(())
}

fn digest(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg("--").arg(path).output()?;
    let text = std::str::from_utf8(&output.stdout)?;
    let hash = text.get(..64).unwrap_or("");
    if !output.status.success() || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!("cannot hash {}", path.display()).into());
    }
    Ok(hash.to_owned())
}

fn output_name(path: &str) -> Result<&str> {
    let name = path
        .strip_prefix("/nix/store/")
        .ok_or("output is not a store path")?;
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(format!("invalid output path: {path}").into());
    }
    Ok(name)
}

fn tree_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            tree_files(&entry.path(), files)?;
        }
    } else if metadata.is_file() {
        files.push(path.to_owned());
    } else {
        return Err(format!("unsupported file: {}", path.display()).into());
    }
    Ok(())
}

fn is_elf(path: &Path) -> Result<bool> {
    let mut header = [0; 4];
    Ok(fs::File::open(path)?.read(&mut header)? == 4 && &header == b"\x7fELF")
}

fn is_notice_file(file: &Path) -> bool {
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    [
        "license",
        "licence",
        "unlicense",
        "copying",
        "copyright",
        "notice",
        "authors",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || name.ends_with(".license")
}

fn notice_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    tree_files(root, &mut files)?;
    files.retain(|file| is_notice_file(file));
    Ok(files)
}

fn source_obligation(license: &str) -> Option<&'static str> {
    if license == "GPL-2.0-only WITH GCC-exception-2.0" {
        Some(
            "The linking exception permits distributing compiled combinations under other terms. Preserve COPYING and the exception. GPLv2 still applies to modifications and distribution outside the exception; retain the exact library source and patches for eon-igae.15.7.",
        )
    } else if license == "GPL-3.0-or-later WITH GCC-exception-3.1" {
        Some(
            "Provide corresponding source for the distributed GCC runtime itself. Preserve the exception; confirm its eligible compilation conditions for the binaries using it.",
        )
    } else if license.starts_with("GPL-") || license.starts_with("AGPL-") {
        Some(
            "Provide complete corresponding source, including modifications and scripts that control compilation and installation, by a method the license permits.",
        )
    } else if license.starts_with("LGPL-") {
        Some(
            "Provide the exact covered library source and modifications. Verify that users can use a modified library through suitable shared linking or provide the materials and instructions needed to relink.",
        )
    } else if license == "MPL-2.0" {
        Some(
            "Tell recipients where to obtain the exact MPL-covered source and modifications, under the MPL.",
        )
    } else {
        None
    }
}

fn validate_native(
    report: &serde_json::Value,
    entry: &Entry,
    exclusions: &BTreeMap<String, NativeExclusion>,
) -> Result<BTreeMap<(String, String), BTreeSet<String>>> {
    let packages: Vec<NativePackage> = serde_json::from_value(
        report
            .get("native_packages")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    for record in report["crates"].as_array().ok_or("missing Cargo crates")? {
        let package = &record["package"];
        let (name, version) = cargo_identity(package)?;
        let c_builder = package["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|dependency| {
                dependency["kind"] == "build"
                    && matches!(
                        dependency["name"].as_str(),
                        Some("cc" | "cmake" | "autotools" | "cxx-build")
                    )
            });
        if (name.ends_with("-sys")
            || name.ends_with("_sys")
            || package["links"].as_str().is_some()
            || c_builder)
            && !entry
                .native_bindings
                .iter()
                .any(|b| b.name == name && b.version == version)
            && !exclusions.get(&name).is_some_and(|e| {
                e.versions.contains(&version)
                    && !e.reason.trim().is_empty()
                    && package["source"] == "registry+https://github.com/rust-lang/crates.io-index"
                    && packages.iter().any(|p| {
                        p.name == name
                            && p.version == version
                            && e.forbidden_features.is_disjoint(&p.features)
                    })
            })
        {
            return Err(format!("unclassified native dependency {name} {version}").into());
        }
    }
    for binding in &entry.native_bindings {
        let original = report["crates"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| {
                r["package"]["name"] == binding.name && r["package"]["version"] == binding.version
            })
            .ok_or_else(|| {
                format!(
                    "native notice {} lacks its runtime Rust binding",
                    binding.notice
                )
            })?;
        let source = original["package"]["source"]
            .as_str()
            .ok_or("native Rust binding has no source")?;
        if source != "registry+https://github.com/rust-lang/crates.io-index"
            && !entry.cargo_source_licenses.contains_key(source)
        {
            return Err(format!(
                "native notice {} has an unreviewed binding source",
                binding.notice
            )
            .into());
        }
        let package = packages
            .iter()
            .find(|p| p.name == binding.name && p.version == binding.version)
            .ok_or_else(|| {
                format!(
                    "native notice {} lacks {} {} in the Cargo graph",
                    binding.notice, binding.name, binding.version
                )
            })?;
        if !binding.features.is_subset(&package.features)
            || !binding.forbidden_features.is_disjoint(&package.features)
            || binding
                .build_package
                .as_ref()
                .is_some_and(|(name, version)| {
                    !packages.iter().any(|p| {
                        &p.name == name
                            && &p.version == version
                            && p.origin == format!("{name} v{version}")
                    })
                })
        {
            return Err(format!(
                "native notice {} does not match the selected build/features",
                binding.notice
            )
            .into());
        }
    }
    let binaries: Vec<NativeBinary> = serde_json::from_value(
        report
            .get("native_binaries")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    let mut notices: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for binary in binaries {
        let native = notices.entry(binary.root).or_default();
        for binding in &entry.native_bindings {
            if binary
                .packages
                .contains(&(binding.name.clone(), binding.version.clone()))
            {
                native.insert(binding.notice.clone());
            }
        }
    }
    Ok(notices)
}

fn notice_obligation(license: &str) -> &'static str {
    match license {
        "0BSD" | "CC0-1.0" | "Unlicense" | "blessing" | "MIT-0" | "LicenseRef-TZ-Public-Domain" => {
            "No mandatory binary attribution; retain the original terms here for provenance."
        }
        "Apache-2.0" | "Apache-2.0 WITH LLVM-exception" | "Apache-2.0 WITH Swift-exception" => {
            "Include the license and upstream NOTICE attribution; mark modified files as applicable. Check the stated exception for embedded portions."
        }
        "OFL-1.1" => {
            "Include font copyright and OFL terms. Preserve OFL for the font, honor reserved font names on modification, and do not sell the font by itself."
        }
        "Bitstream-Vera" | "Arev" | "AMS" => {
            "Preserve original font notices and restrictions on renaming, standalone sale or endorsement where specified."
        }
        "OpenSSL" => {
            "Keep both original OpenSSL and SSLeay terms and acknowledgements, including applicable acknowledgements in advertising."
        }
        "CDLA-Permissive-2.0" => "Provide the agreement text with the redistributed data.",
        "CC-PDDC" => {
            "Retain the original public domain notice and source identity for the approved pwd release."
        }
        "CC-BY-4.0" => {
            "Credit the original authors, identify the source and license, and indicate adaptations such as remapping glyphs. Preserve the recipients' rights under the license."
        }
        "LicenseRef-Accelerando" => {
            "Use only the attribution and recipient terms in the applicable redistribution grant."
        }
        license if source_obligation(license).is_some() => {
            "Retain the applicable copyright, license and warranty notices; mark changes as required. See source obligations for this component."
        }
        _ => {
            "Keep the original copyright, terms and disclaimers; honor any restrictions on use of names or endorsement."
        }
    }
}

fn record_source(
    obligations: &mut BTreeMap<String, BTreeMap<String, String>>,
    component: String,
    license: &str,
) {
    if let Some(obligation) = source_obligation(license) {
        obligations
            .entry(component)
            .or_default()
            .insert(license.to_owned(), obligation.to_owned());
    }
}

fn cargo_original_terms(report: &Path, crate_index: usize) -> Result<bool> {
    let directory = report
        .parent()
        .ok_or("Cargo report has no parent")?
        .join("texts")
        .join(crate_index.to_string());
    if !directory.is_dir() {
        return Ok(false);
    }
    for file in notice_files(&directory)? {
        let relative = file.strip_prefix(&directory)?;
        let first = relative.components().next().ok_or("empty notice path")?;
        let name = first.as_os_str().to_string_lossy().to_ascii_lowercase();
        if ["license", "licence", "unlicense", "copying", "copyright"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            && fs::metadata(file)?.len() > 0
        {
            return Ok(true);
        }
    }
    Ok(false)
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "eon-notice-sources-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn source(&self, index: usize, path: &Path) -> Result<PathBuf> {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir()
            || !(name.contains(".tar.") || name.ends_with(".tar") || name.ends_with(".tgz"))
        {
            return Ok(path.to_owned());
        }
        let target = self.0.join(index.to_string());
        fs::create_dir(&target)?;
        let listing = Command::new("tar")
            .args(["--quoting-style=literal", "-tf"])
            .arg(path)
            .output()?;
        if !listing.status.success() {
            return Err(format!("cannot list notice source {}", path.display()).into());
        }
        let mut notices = Vec::new();
        for member in std::str::from_utf8(&listing.stdout)?.lines() {
            if !member.ends_with('/') && is_notice_file(Path::new(member)) {
                notices.extend_from_slice(member.as_bytes());
                notices.push(0);
            }
        }
        if notices.is_empty() {
            return Ok(target);
        }
        let members = self.0.join(format!("{index}.members"));
        fs::write(&members, notices)?;
        let result = Command::new("tar")
            .args([
                "--no-same-owner",
                "--no-same-permissions",
                "--no-recursion",
                "-xf",
            ])
            .arg(path)
            .arg("-C")
            .arg(&target)
            .args(["--null", "-T"])
            .arg(&members)
            .status()?;
        if !result.success() {
            return Err(format!("cannot unpack notice source {}", path.display()).into());
        }
        Ok(target)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn generate(input: &Path, tree: &Path, output: &Path) -> Result<()> {
    let mut inputs: Inputs = serde_json::from_slice(&fs::read(input)?)?;
    if inputs.schema != 1 {
        return Err("unsupported input schema".into());
    }
    // Never overwrite an earlier report or a user-owned directory.
    fs::create_dir(output)?;
    fs::create_dir(output.join("texts"))?;
    let mut findings = Vec::new();
    let mut notice_gaps = Vec::new();
    let mut owners = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut files = Vec::new();
    let mut binaries = BTreeMap::new();
    let mut notice_obligations = BTreeMap::new();
    let mut source_obligations = BTreeMap::new();
    let store = tree.join("nix/store");
    let actual = fs::read_dir(&store)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("non-UTF-8 store name"))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    for entry in &mut inputs.entries {
        if let Some(name) = &entry.output_name {
            entry.outputs = actual
                .iter()
                .filter(|path| path.get(33..) == Some(name.as_str()))
                .map(|path| format!("/nix/store/{path}"))
                .collect();
        }
        entry.outputs.retain(|path| {
            output_name(path).is_ok_and(|name| actual.iter().any(|output| output == name))
        });
    }
    let mut needed: BTreeSet<_> = inputs
        .entries
        .iter()
        .filter(|entry| {
            entry
                .outputs
                .iter()
                .any(|path| output_name(path).is_ok_and(|name| actual.iter().any(|p| p == name)))
        })
        .map(|entry| entry.id.clone())
        .collect();
    needed.extend(inputs.binary_inputs.values().flatten().cloned());
    loop {
        let before = needed.len();
        for entry in &inputs.entries {
            if needed.contains(&entry.id) {
                needed.extend(entry.embedded.iter().cloned());
                needed.extend(entry.native_bindings.iter().map(|b| b.notice.clone()));
            }
        }
        if before == needed.len() {
            break;
        }
    }
    inputs.entries.retain(|entry| needed.contains(&entry.id));
    let scratch = Scratch::new()?;
    let mut source_notices = BTreeMap::new();
    let mut all_notices = Vec::new();
    for entry in &inputs.entries {
        let mut notices = entry.notice_files.clone();
        for root in &entry.source_roots {
            if !source_notices.contains_key(root) {
                let directory = scratch.source(source_notices.len(), root)?;
                source_notices.insert(root.clone(), (notice_files(&directory)?, directory));
            }
            notices.extend(source_notices[root].0.iter().cloned());
        }
        notices.sort();
        notices.dedup();
        all_notices.push(notices);
    }
    for (index, entry) in inputs.entries.iter().enumerate() {
        let mut native_by_root: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        for problem in &entry.unresolved {
            findings.push(format!("{}: {problem}", entry.id));
        }
        for (file, expected) in &entry.notice_hashes {
            if digest(file)? != *expected {
                findings.push(format!(
                    "{}: changed license evidence {}",
                    entry.id,
                    file.display()
                ));
            }
        }
        for file in &entry.checks {
            if fs::read_to_string(file)?.trim() != "passed" {
                findings.push(format!(
                    "{}: dependency notice check failed {}",
                    entry.id,
                    file.display()
                ));
            }
        }
        if entry.id.trim().is_empty() || !ids.insert(entry.id.clone()) {
            findings.push(format!("empty or duplicate entry: {}", entry.id));
        }
        if entry.source.trim().is_empty() || entry.version.trim().is_empty() {
            findings.push(format!("{}: missing source/version", entry.id));
        }
        covered.extend(entry.components.iter().cloned());
        if entry.licenses.is_empty() {
            findings.push(format!("{}: missing license", entry.id));
        }
        for license in &entry.licenses {
            notice_obligations.insert(license.clone(), notice_obligation(license).to_owned());
            record_source(&mut source_obligations, entry.id.clone(), license);
            if !inputs.accepted.contains(license) {
                findings.push(format!("{}: unaccepted license {license}", entry.id));
            }
            if entry.grammar.is_some()
                && (license.starts_with("GPL-") || license.starts_with("AGPL-"))
            {
                findings.push(format!(
                    "{}: strong-copyleft grammar must be omitted",
                    entry.id
                ));
            }
        }
        if let Some(grammar) = &entry.grammar
            && inputs.omitted_grammars.contains(grammar)
        {
            findings.push(format!(
                "{}: omitted grammar has a shipped notice entry",
                entry.id
            ));
        }
        if entry
            .licenses
            .iter()
            .any(|license| license == "LicenseRef-Accelerando")
        {
            match &entry.grant {
                None => findings.push(format!("{}: missing redistribution grant", entry.id)),
                Some(grant) => {
                    let fields = [
                        &grant.rights_holder,
                        &grant.recipients_rights,
                        &grant.attribution,
                        &grant.revocability,
                    ];
                    if fields.iter().any(|field| field.trim().is_empty())
                        || digest(&grant.record)? != grant.sha256
                        || grant.assets.len() != 3
                        || !grant.public_binary_redistribution
                        || !grant.modified_regular_font
                        || !grant.modified_bold_font
                        || !grant.bitmap_glyphs
                    {
                        findings.push(format!("{}: incomplete or changed grant", entry.id));
                    }
                    for (asset, expected) in &grant.assets {
                        if digest(asset)? != *expected {
                            findings.push(format!(
                                "{}: grant does not cover asset {}",
                                entry.id,
                                asset.display()
                            ));
                        }
                    }
                }
            }
        }
        if all_notices[index].is_empty() && entry.cargo_reports.is_empty() {
            findings.push(format!("{}: missing notice material", entry.id));
        }
        for (number, notice) in all_notices[index].iter().enumerate() {
            let bytes = fs::read(notice)?;
            let source = source_notices
                .iter()
                .find_map(|(root, (_, directory))| {
                    if root == directory {
                        return None;
                    }
                    let relative = notice.strip_prefix(directory).ok()?;
                    Some(format!("{}:{}", root.display(), relative.display()))
                })
                .unwrap_or_else(|| notice.display().to_string());
            if bytes.iter().all(u8::is_ascii_whitespace) {
                findings.push(format!("{}: empty notice {source}", entry.id));
            }
            let name = format!("texts/{index}-{number}.txt");
            fs::write(output.join(&name), bytes)?;
            files.push(File {
                owner: entry.id.clone(),
                source,
                sha256: digest(&output.join(&name))?,
                path: name,
            });
        }
        for (number, source) in entry.cargo_reports.iter().enumerate() {
            let report: serde_json::Value = serde_json::from_slice(&fs::read(source)?)?;
            let empty = Vec::new();
            let licenses = report["licenses"].as_array().unwrap_or(&empty);
            let crates = report["crates"].as_array().unwrap_or(&empty);
            if licenses.is_empty() || crates.is_empty() {
                findings.push(format!("{}: empty Cargo report", entry.id));
            } else {
                match validate_native(&report, entry, &inputs.native_exclusions) {
                    Ok(native) => {
                        for (root, dependencies) in native {
                            native_by_root.entry(root).or_default().extend(dependencies);
                        }
                    }
                    Err(error) => findings.push(format!("{}: {error}", entry.id)),
                }
            }
            covered.extend(
                crates
                    .iter()
                    .filter_map(|info| info["package"]["name"].as_str())
                    .map(str::to_owned),
            );
            for license in licenses {
                let id = license["id"]
                    .as_str()
                    .ok_or("Cargo license has no SPDX id")?;
                notice_obligations.insert(id.to_owned(), notice_obligation(id).to_owned());
                if !inputs.accepted.contains(id) && id != "CC-PDDC" {
                    findings.push(format!("{}: unaccepted Cargo license {id}", entry.id));
                }
                for usage in license["used_by"].as_array().into_iter().flatten() {
                    let package = &usage["crate"];
                    let name = package["name"]
                        .as_str()
                        .ok_or("missing Cargo package name")?;
                    let version = package["version"]
                        .as_str()
                        .ok_or("missing Cargo package version")?;
                    let component = format!("{} / {name} {version}", entry.id);
                    record_source(&mut source_obligations, component.clone(), id);
                    let crate_index = crates
                        .iter()
                        .position(|record| record["package"]["id"] == package["id"]);
                    if id == "CC-PDDC" {
                        let approved = if let Some(index) = crate_index {
                            name == "pwd"
                                && version == "1.4.0"
                                && entry.components.contains("nushell")
                                && package["repository"] == "https://gitlab.com/pwoolcoc/pwd.git"
                                && crates[index]["source_revision"]
                                    == "2ce52c5c855f9d116a4a746aaa39030a50bcd046"
                                && digest(&source.with_file_name(format!("texts/{index}/LICENSE")))?
                                    == "88a49ec7b8c2a0ab11d597889a39e2bb631eee8c3b184d1d0dbd2ea15388bb9b"
                        } else {
                            false
                        };
                        if !approved {
                            findings.push(format!("{component}: unaccepted CC-PDDC license"));
                        }
                    }
                    let uses_package_terms = package["source"].is_null()
                        && entry.licenses.iter().any(|license| license == id)
                        && !all_notices[index].is_empty();
                    let uses_pinned_source_terms = package["source"]
                        .as_str()
                        .and_then(|source| entry.cargo_source_licenses.get(source))
                        .is_some_and(|licenses| licenses.iter().any(|license| license == id));
                    if license["source_path"].is_null()
                        && !uses_package_terms
                        && !uses_pinned_source_terms
                    {
                        let has_terms = match crate_index {
                            Some(index) => cargo_original_terms(source, index)?,
                            None => false,
                        };
                        if !has_terms {
                            match &entry.upstream_distribution {
                                Some(distributor) if source_obligation(id).is_none() => {
                                    notice_gaps.push(format!("{component}: Cargo metadata declares {id}; the vendored source lacks the original notice. This crate is distributed as part of {distributor}."));
                                }
                                _ => findings
                                    .push(format!("{component}: missing original {id} notice")),
                            }
                        }
                    }
                }
            }
            let name = format!("texts/{index}-cargo-{number}.json");
            fs::write(output.join(&name), serde_json::to_vec_pretty(&report)?)?;
            files.push(File {
                owner: entry.id.clone(),
                source: source.display().to_string(),
                sha256: digest(&output.join(&name))?,
                path: name,
            });
        }
        let forbidden = entry
            .forbidden_embedded_files
            .iter()
            .map(fs::read)
            .collect::<std::io::Result<Vec<_>>>()?;
        for path in &entry.outputs {
            let name = output_name(path)?;
            if owners.insert(name.to_owned(), &entry.id).is_some() {
                findings.push(format!("duplicate output owner: {path}"));
            }
            if !store.join(name).exists() {
                continue;
            }
            let mut contained = Vec::new();
            tree_files(&store.join(name), &mut contained)?;
            for file in contained {
                if !is_elf(&file)? {
                    continue;
                }
                let relative = file
                    .strip_prefix(tree)?
                    .to_str()
                    .ok_or("non-UTF-8 binary path")?
                    .to_owned();
                let embedded = inputs
                    .binary_inputs
                    .get(&relative)
                    .unwrap_or(&entry.embedded);
                let mut native_dependencies = BTreeSet::new();
                if !entry.native_bindings.is_empty()
                    && !inputs.binary_inputs.contains_key(&relative)
                {
                    let packages = audit_packages(&file)?;
                    for root in packages
                        .iter()
                        .filter(|p| p["root"].as_bool() == Some(true))
                    {
                        match native_by_root.get(&cargo_identity(root)?) {
                            Some(dependencies) => {
                                native_dependencies.extend(dependencies.iter().cloned())
                            }
                            None => findings.push(format!(
                                "{}: {relative} has no native Cargo-root binding",
                                entry.id
                            )),
                        }
                    }
                }
                if embedded.is_empty() {
                    findings.push(format!(
                        "{}: unaccounted static content in {}",
                        entry.id,
                        file.display()
                    ));
                }
                let dependencies = embedded.iter().filter_map(|id| {
                    inputs
                        .entries
                        .iter()
                        .find(|dependency| &dependency.id == id)
                });
                let rust = dependencies.filter(|dependency| {
                    dependency.id.starts_with("rust-")
                        && dependency.id.ends_with("standard-library")
                });
                for stdlib in rust {
                    let comments = Command::new("readelf")
                        .args(["-p", ".comment"])
                        .arg(&file)
                        .output()?;
                    if !String::from_utf8_lossy(&comments.stdout)
                        .contains(&format!("rustc version {} (", stdlib.version))
                    {
                        findings.push(format!(
                            "{}: {relative} lacks evidence of Rust {}",
                            entry.id, stdlib.version
                        ));
                    }
                }
                if !forbidden.is_empty() {
                    let bytes = fs::read(&file)?;
                    if forbidden.iter().any(|pattern| {
                        pattern.is_empty()
                            || bytes.windows(pattern.len()).any(|part| part == pattern)
                    }) {
                        findings.push(format!(
                            "{}: excluded artwork remains in {relative}",
                            entry.id
                        ));
                    }
                }
                binaries.insert(
                    relative.clone(),
                    Binary {
                        owner: entry.id.clone(),
                        sha256: digest(&file)?,
                        embedded: embedded.clone(),
                        native_dependencies: native_dependencies.into_iter().collect(),
                    },
                );
            }
        }
    }
    for entry in &inputs.entries {
        for id in &entry.embedded {
            if !ids.contains(id) {
                findings.push(format!("{}: missing embedded dependency {id}", entry.id));
            }
        }
        for binding in &entry.native_bindings {
            if !ids.contains(&binding.notice) {
                findings.push(format!(
                    "{}: missing native notice {}",
                    entry.id, binding.notice
                ));
            }
        }
    }
    for missing in inputs.required_components.difference(&covered) {
        findings.push(format!("component lacks notice ownership: {missing}"));
    }
    for (path, deps) in &inputs.binary_inputs {
        if !binaries.contains_key(path) || deps.iter().any(|id| !ids.contains(id)) {
            findings.push(format!("unmatched binary or static inputs: {path}"));
        }
    }
    for path in fs::read_dir(&store)? {
        let path = path?;
        let name = path
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 store name")?;
        if !owners.contains_key(&name) {
            findings.push(format!("unknown shipped output: {name}"));
        }
        for grammar in &inputs.omitted_grammars {
            if name.contains(&format!("-helix-tree-sitter-{grammar}-")) {
                findings.push(format!("omitted grammar is present: {grammar}"));
            }
        }
    }
    findings.sort();
    findings.dedup();
    notice_gaps.sort();
    notice_gaps.dedup();
    let report = Report {
        schema: 1,
        entries: inputs.entries,
        files,
        binaries,
        notice_obligations,
        source_obligations,
        notice_gaps,
        findings,
    };
    fs::write(
        output.join("inventory.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        output.join("README.txt"),
        "Eon archive third-party notices\n\nSee inventory.json for component/source identities, per-binary static inputs, original texts, notice gaps and summaries of source obligations.\nThis report is generated from the archive producers and the actual relinked tree.\nA nonempty findings list blocks publication. Source delivery is separately gated (eon-igae.15.7).\nRelinked ELF interpreters and library paths and absolute symlinks are modified by Eon.\n",
    )?;
    Ok(())
}

fn check(output: &Path) -> Result<()> {
    let report: Report = serde_json::from_slice(&fs::read(output.join("inventory.json"))?)?;
    if report.schema != 1 {
        return Err("unsupported report schema".into());
    }
    for file in &report.files {
        if !Path::new(&file.path).is_relative()
            || Path::new(&file.path)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("notice path escapes report".into());
        }
        if digest(&output.join(&file.path))? != file.sha256 {
            return Err(format!("notice is missing or changed: {}", file.path).into());
        }
    }
    if !report.findings.is_empty() {
        return Err(report.findings.join("\n").into());
    }
    if report.entries.is_empty() || report.files.is_empty() {
        return Err("empty notice report".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "eon-notices-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::create_dir_all(path.join("tree/nix/store/hash-tool-1")).unwrap();
            fs::write(
                path.join("LICENSE"),
                "Copyright Example\nPermission to redistribute.\n",
            )
            .unwrap();
            Self(path)
        }
        fn input(&self) -> serde_json::Value {
            json!({"schema":1,"accepted":["MIT"],"required_components":["tool"],"omitted_grammars":["drools"],
                "entries":[{"id":"tool","version":"1","source":"pinned source","licenses":["MIT"],"components":["tool"],
                    "outputs":["/nix/store/hash-tool-1"],"notice_files":[self.0.join("LICENSE")]}]})
        }
        fn generate(&self, input: serde_json::Value) -> Report {
            fs::write(
                self.0.join("input.json"),
                serde_json::to_vec(&input).unwrap(),
            )
            .unwrap();
            generate(
                &self.0.join("input.json"),
                &self.0.join("tree"),
                &self.0.join("notices"),
            )
            .unwrap();
            serde_json::from_slice(&fs::read(self.0.join("notices/inventory.json")).unwrap())
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn source_archive_extracts_only_notice_files() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("source.rs"), "fn main() {}").unwrap();
        fs::write(fixture.0.join("UNLICENSE"), "public-domain dedication").unwrap();
        fs::write(fixture.0.join("AUTHORS.txt"), "original authors").unwrap();
        let archive = fixture.0.join("source.tar");
        assert!(
            Command::new("tar")
                .args(["-cf"])
                .arg(&archive)
                .arg("-C")
                .arg(&fixture.0)
                .args(["LICENSE", "UNLICENSE", "AUTHORS.txt", "source.rs"])
                .status()
                .unwrap()
                .success()
        );
        let scratch = Scratch::new().unwrap();
        let root = scratch.source(0, &archive).unwrap();
        for file in ["LICENSE", "UNLICENSE", "AUTHORS.txt"] {
            assert_eq!(
                fs::read(root.join(file)).unwrap(),
                fs::read(fixture.0.join(file)).unwrap()
            );
        }
        assert!(!root.join("source.rs").exists());
    }

    #[test]
    fn unlicense_is_original_terms_and_authors_is_only_attribution() {
        let fixture = Fixture::new();
        let directory = fixture.0.join("texts/0");
        fs::create_dir_all(&directory).unwrap();
        let report = fixture.0.join("report.json");
        fs::write(directory.join("AUTHORS"), "original authors").unwrap();
        assert!(!cargo_original_terms(&report, 0).unwrap());
        fs::write(directory.join("UNLICENSE"), "public-domain dedication").unwrap();
        assert!(cargo_original_terms(&report, 0).unwrap());
    }

    #[test]
    fn original_notice_gap_for_stock_crate_does_not_waive_copyleft() {
        for license in ["MIT", "MPL-2.0"] {
            let fixture = Fixture::new();
            let mut input = fixture.input();
            let source = fixture.0.join("cargo.json");
            let package = json!({"name":"dependency","version":"1.0.0","id":"locked",
                "source":"registry+https://github.com/rust-lang/crates.io-index"});
            let cargo = json!({"crates":[{"package":package}],"licenses":[
                {"id":license,"source_path":null,"used_by":[{"crate":package}]}
            ]});
            fs::write(&source, serde_json::to_vec(&cargo).unwrap()).unwrap();
            input["accepted"] = json!(["MIT", "MPL-2.0"]);
            input["entries"][0]["cargo_reports"] = json!([source]);
            input["entries"][0]["upstream_distribution"] = json!("the pinned distributor build");
            let report = fixture.generate(input);
            if license == "MIT" {
                assert_eq!(report.notice_gaps.len(), 1);
                assert!(report.findings.is_empty());
                check(&fixture.0.join("notices")).unwrap();
            } else {
                assert!(report.notice_gaps.is_empty());
                assert_eq!(report.findings.len(), 1);
                assert!(check(&fixture.0.join("notices")).is_err());
            }
        }
    }

    #[test]
    fn notice_removal_unknown_output_and_missing_component_block_publication() {
        let fixture = Fixture::new();
        let mut input = fixture.input();
        input["entries"][0]["outputs"] = json!([
            "/nix/store/hash-tool-1",
            "/nix/store/hash-unshipped-development-output"
        ]);
        let report = fixture.generate(input);
        assert_eq!(report.entries[0].outputs, ["/nix/store/hash-tool-1"]);
        assert!(report.findings.is_empty());
        check(&fixture.0.join("notices")).unwrap();
        fs::remove_file(fixture.0.join("notices").join(&report.files[0].path)).unwrap();
        assert!(check(&fixture.0.join("notices")).is_err());

        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("tree/nix/store/hash-surprise-1")).unwrap();
        let mut input = fixture.input();
        input["required_components"] = json!(["tool", "missing-tool"]);
        let report = fixture.generate(input);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.contains("unknown shipped output"))
        );
        assert!(report.findings.iter().any(|f| f.contains("missing-tool")));
        assert!(check(&fixture.0.join("notices")).is_err());
    }

    #[test]
    fn omitted_grammar_unlisted_strong_copyleft_and_missing_grant_fail_closed() {
        let fixture = Fixture::new();
        fs::create_dir(
            fixture
                .0
                .join("tree/nix/store/hash-helix-tree-sitter-drools-revision"),
        )
        .unwrap();
        let mut input = fixture.input();
        input["entries"][0]["grammar"] = json!("another-grammar");
        input["entries"][0]["licenses"] = json!(["AGPL-3.0-only", "LicenseRef-Accelerando"]);
        let report = fixture.generate(input);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.contains("omitted grammar is present"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.contains("strong-copyleft grammar"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.contains("missing redistribution grant"))
        );
        assert!(check(&fixture.0.join("notices")).is_err());

        let fixture = Fixture::new();
        let mut input = fixture.input();
        input["accepted"] = json!(["LicenseRef-TZ-Public-Domain"]);
        input["entries"][0]["licenses"] = json!(["LicenseRef-TZ-Public-Domain"]);
        assert!(fixture.generate(input).findings.is_empty());
        check(&fixture.0.join("notices")).unwrap();
    }

    #[test]
    fn every_elf_needs_static_inputs_bound_to_real_dependency_entries() {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join("tree/nix/store/hash-tool-1/program"),
            b"\x7fELFtest",
        )
        .unwrap();
        let mut input = fixture.input();
        input["entries"][0]["embedded"] = json!(["missing-runtime"]);
        let report = fixture.generate(input);
        assert_eq!(report.binaries.len(), 1);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.contains("missing embedded dependency"))
        );
        assert!(check(&fixture.0.join("notices")).is_err());
    }

    #[test]
    fn go_dependencies_need_the_exact_vendor_version_and_a_license_entry() {
        let build =
            br#"{"GoVersion":"go1.26.3","Deps":[{"Path":"example.org/lib","Version":"v1.2.3"}]}"#;
        let modules = "# example.org/lib v1.2.3\n## explicit\nexample.org/lib/p";
        let licenses = "example.org/lib/p,https://example.org/LICENSE,MIT\n";
        check_go(build, modules, licenses, "1.26.3").unwrap();
        assert!(check_go(build, "# example.org/lib v1.2.30", licenses, "1.26.3").is_err());
        assert!(check_go(build, modules, "example.org/library,u,MIT", "1.26.3").is_err());
        let replaced = br#"{"GoVersion":"go1.26.3","Deps":[{"Path":"example.org/lib","Version":"v1.2.3","Replace":{"Path":"example.net/lib","Version":"v1.2.4"}}]}"#;
        check_go(
            replaced,
            "# example.org/lib v1.2.3 => example.net/lib v1.2.4",
            "example.net/lib/p,u,MIT",
            "1.26.3",
        )
        .unwrap();
        assert!(check_go(replaced, modules, licenses, "1.26.3").is_err());
    }

    #[test]
    fn supplementary_original_notices_require_the_exact_crate_revision() {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join(".cargo_vcs_info.json"),
            br#"{"git":{"sha1":"revision"}}"#,
        )
        .unwrap();
        let file = fixture.0.join("LICENSE");
        fs::write(&file, "original copyright and terms").unwrap();
        let mut notice = CargoNotice {
            version: "1.0.0".into(),
            revision: "revision".into(),
            files: vec![file],
        };
        let package = json!({"name":"crate", "version":"1.0.0"});
        let output = fixture.0.join("supplement");
        supplement_cargo_notice(&package, &fixture.0, &output, &notice).unwrap();
        assert_eq!(
            fs::read_to_string(output.join("LICENSE-upstream-0")).unwrap(),
            "original copyright and terms"
        );
        notice.revision = "different".into();
        assert!(supplement_cargo_notice(&package, &fixture.0, &output, &notice).is_err());
    }

    #[test]
    fn c_build_dependencies_require_native_classification_without_sys_names_or_links() {
        let fixture = Fixture::new();
        let entry: Entry = serde_json::from_value(fixture.input()["entries"][0].clone()).unwrap();
        for builder in ["cc", "cmake", "autotools", "cxx-build"] {
            let report = json!({"crates":[{"package":{
                "name":"tree-house-bindings", "version":"0.3.2", "links":null,
                "dependencies":[{"name":builder,"rename":"native_compiler","kind":"build"}]
            }}]});
            assert!(
                validate_native(&report, &entry, &BTreeMap::new())
                    .unwrap_err()
                    .to_string()
                    .contains("unclassified native dependency tree-house-bindings 0.3.2")
            );
        }
        let mut report = json!({
            "crates":[{"package":{
                "name":"av-scenechange", "version":"0.14.1",
                "source":"registry+https://github.com/rust-lang/crates.io-index",
                "dependencies":[{"name":"cc","kind":"build"}]
            }}],
            "native_packages":[{"name":"av-scenechange","version":"0.14.1",
                "origin":"av-scenechange v0.14.1","features":[]}]
        });
        let exclusions = serde_json::from_value(json!({"av-scenechange":{
            "versions":["0.14.1"], "reason":"assembly is disabled",
            "forbidden_features":["asm"]
        }}))
        .unwrap();
        validate_native(&report, &entry, &exclusions).unwrap();
        report["native_packages"][0]["features"] = json!(["asm"]);
        assert!(validate_native(&report, &entry, &exclusions).is_err());
    }

    #[test]
    fn native_graphs_merge_features_and_native_notices_follow_actual_binary_roots() {
        let packages = native_packages("mlua-sys v0.10.0 features=lua55\nmlua-sys v0.10.0 features=vendored (*)\nlua-src v550.0.0 features=\n").unwrap();
        assert!(native_packages("mlua-sys v0.10.0").is_err());
        let fixture = Fixture::new();
        let mut entry: Entry =
            serde_json::from_value(fixture.input()["entries"][0].clone()).unwrap();
        entry.native_bindings = vec![
            serde_json::from_value(json!({
                "name":"mlua-sys","version":"0.10.0","notice":"lua",
                "features":["lua55","vendored"],"build_package":["lua-src","550.0.0"]
            }))
            .unwrap(),
        ];
        let mut report = json!({"native_packages":packages,"crates":[{"package":{
            "name":"mlua-sys","version":"0.10.0","source":"registry+https://github.com/rust-lang/crates.io-index"
        }}],"native_binaries":[
            {"root":["application","1"],"packages":[["mlua-sys","0.10.0"]]},
            {"root":["auxiliary","1"],"packages":[["auxiliary","1"]]}
        ]});
        let native = validate_native(&report, &entry, &BTreeMap::new()).unwrap();
        assert_eq!(
            native[&("application".into(), "1".into())],
            BTreeSet::from(["lua".into()])
        );
        assert!(native[&("auxiliary".into(), "1".into())].is_empty());
        report["native_packages"][1]["features"] = json!(["lua54", "vendored"]);
        assert!(validate_native(&report, &entry, &BTreeMap::new()).is_err());
        entry.native_bindings.clear();
        assert!(validate_native(&report, &entry, &BTreeMap::new()).is_err());
    }
}
