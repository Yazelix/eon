use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    product: Product,
    composition: Composition,
    activation: Activation,
    components: Vec<Component>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Product {
    id: String,
    targets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Composition {
    services: Vec<String>,
    clients: Vec<String>,
    tools: Vec<String>,
    libraries: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    required_contracts: Vec<RequiredContract>,
    required_interfaces: Vec<RequiredInterface>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredContract {
    component: String,
    contract: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredInterface {
    component: String,
    interface: String,
    version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    id: String,
    project: String,
    version: String,
    revision: Option<String>,
    targets: Vec<String>,
    source: Source,
    artifacts: Vec<Artifact>,
    contracts: Vec<Contract>,
    interfaces: Vec<Interface>,
    requires: Vec<Requirement>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Source {
    Git { url: String },
    Workspace { path: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    id: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    id: String,
    proof: String,
}

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Interface {
    id: String,
    version: u32,
    proof: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Requirement {
    component: String,
    interfaces: Vec<Interface>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub fn parse_and_validate(input: &str) -> Result<(), Error> {
    validate(&parse(input)?)
}

// Eon's approved platforms: each graph target with the Rust architecture and
// operating system that build it.
const TARGETS: [(&str, &str, &str); 2] = [
    ("x86_64-linux", "x86_64", "linux"),
    ("aarch64-darwin", "aarch64", "macos"),
];

/// Reports the product and the components this build's target carries.
pub fn version_report(input: &str) -> Result<String, Error> {
    let target = TARGETS
        .iter()
        .find(|(_, arch, os)| (*arch, *os) == (std::env::consts::ARCH, std::env::consts::OS))
        .ok_or_else(|| Error("unsupported build target".into()))?
        .0;
    report(input, target)
}

fn report(input: &str, target: &str) -> Result<String, Error> {
    let manifest = parse(input)?;
    validate(&manifest)?;
    required(
        manifest.product.targets.iter().any(|name| name == target),
        "unsupported build target",
    )?;
    let mut lines = vec![format!("{} {target}", manifest.product.id)];
    lines.extend(
        manifest
            .components
            .iter()
            .filter(|component| component.targets.iter().any(|name| name == target))
            .map(|component| {
                format!(
                    "{} {} {} {target}",
                    component.id,
                    component.version,
                    component.revision.as_deref().unwrap_or("workspace"),
                )
            }),
    );
    Ok(lines.join("\n"))
}

fn parse(input: &str) -> Result<Manifest, Error> {
    let decoded: serde_json::Value = serde_json::from_str(input)
        .map_err(|error| Error(format!("invalid manifest JSON: {error}")))?;
    validate_strings(&decoded)?;
    serde_json::from_str(input).map_err(|error| Error(format!("invalid manifest JSON: {error}")))
}

fn validate_strings(value: &serde_json::Value) -> Result<(), Error> {
    match value {
        serde_json::Value::String(value) => {
            required(
                value != "/nix/store" && !value.contains("/nix/store/"),
                "Nix store paths are resolved inputs, not component identity",
            )?;
            required(!value.contains('\0'), "manifest string contains NUL")
        }
        serde_json::Value::Array(values) => values.iter().try_for_each(validate_strings),
        serde_json::Value::Object(values) => values.values().try_for_each(validate_strings),
        _ => Ok(()),
    }
}

fn validate(manifest: &Manifest) -> Result<(), Error> {
    required(manifest.schema == 5, "unsupported manifest schema")?;
    required(token(&manifest.product.id), "invalid product id")?;
    required(
        !manifest.product.targets.is_empty(),
        "product has no targets",
    )?;
    let mut product_targets = HashSet::new();
    for target in &manifest.product.targets {
        required(
            product_targets.insert(target.as_str()),
            "duplicate product target",
        )?;
        required(
            TARGETS.iter().any(|(name, ..)| name == target),
            "unsupported product target",
        )?;
    }
    required(!manifest.components.is_empty(), "component set is empty")?;

    let mut components = HashMap::new();
    for component in &manifest.components {
        required(token(&component.id), "invalid component id")?;
        required(
            !component.project.trim().is_empty(),
            "missing component project",
        )?;
        required(
            !component.version.trim().is_empty(),
            "missing component version",
        )?;
        required(!component.targets.is_empty(), "component has no targets")?;
        let mut targets = HashSet::new();
        for target in &component.targets {
            required(
                targets.insert(target.as_str()),
                "duplicate component target",
            )?;
            required(
                product_targets.contains(target.as_str()),
                "component target is not a product target",
            )?;
        }
        match &component.source {
            Source::Git { url } => {
                required(
                    component.revision.as_deref().is_some_and(revision),
                    "invalid component revision",
                )?;
                required(
                    url.strip_prefix("https://")
                        .and_then(|source| source.split_once('/'))
                        .is_some_and(|(host, path)| {
                            !host.is_empty()
                                && path.rsplit('/').next().is_some_and(|repository| {
                                    repository != ".git" && repository.ends_with(".git")
                                })
                        }),
                    "component source must be an HTTPS Git repository",
                )?;
            }
            // The repository revision identifies in-tree components.
            Source::Workspace { path } => {
                required(
                    component.revision.is_none(),
                    "workspace component cannot carry a revision",
                )?;
                required(
                    path.split('/').all(token),
                    "invalid workspace component path",
                )?;
            }
        }
        required(
            !component.artifacts.is_empty(),
            "component has no artifacts",
        )?;

        let mut artifacts = HashSet::new();
        for artifact in &component.artifacts {
            required(
                artifacts.insert(artifact.id.as_str()),
                "duplicate artifact id",
            )?;
            required(token(&artifact.id), "invalid artifact id")?;
            required(
                matches!(artifact.kind.as_str(), "file" | "cargo-package"),
                "unsupported artifact kind",
            )?;
        }

        let mut contracts = HashSet::new();
        for contract in &component.contracts {
            required(
                contracts.insert(contract.id.as_str()),
                "duplicate contract id",
            )?;
            required(token(&contract.id), "invalid contract id")?;
            required(revision(&contract.proof), "invalid contract proof")?;
        }

        let mut interfaces = HashSet::new();
        for interface in &component.interfaces {
            required(interface.version > 0, "invalid interface version")?;
            required(
                interfaces.insert((interface.id.as_str(), interface.version)),
                "duplicate interface",
            )?;
            required(token(&interface.id), "invalid interface id")?;
            required(revision(&interface.proof), "invalid interface proof")?;
        }

        let mut requirements = HashSet::new();
        for requirement in &component.requires {
            required(
                requirements.insert(requirement.component.as_str()),
                "duplicate component requirement",
            )?;
            required(
                requirement.component != component.id,
                "component cannot require itself",
            )?;
            required(
                !requirement.interfaces.is_empty(),
                "component requirement has no interfaces",
            )?;
            let mut required_interfaces = HashSet::new();
            for interface in &requirement.interfaces {
                required(interface.version > 0, "invalid required interface version")?;
                required(
                    required_interfaces.insert((interface.id.as_str(), interface.version)),
                    "duplicate required interface",
                )?;
                required(token(&interface.id), "invalid required interface id")?;
                required(
                    revision(&interface.proof),
                    "invalid required interface proof",
                )?;
            }
        }

        required(
            components
                .insert(component.id.as_str(), component)
                .is_none(),
            "duplicate component id",
        )?;
    }

    let roles = [
        ("service", &manifest.composition.services),
        ("client", &manifest.composition.clients),
        ("tool", &manifest.composition.tools),
        ("library", &manifest.composition.libraries),
    ];
    let mut component_roles = HashMap::new();
    for (role, members) in roles {
        required(!members.is_empty(), format!("composition has no {role}s"))?;
        for id in members {
            required(
                components.contains_key(id.as_str()),
                "role references missing component",
            )?;
            required(
                component_roles.insert(id.as_str(), role).is_none(),
                "component has multiple roles",
            )?;
        }
    }
    required(
        component_roles.len() == components.len(),
        "component has no composition role",
    )?;

    for component in &manifest.components {
        let role = component_roles[component.id.as_str()];
        if role != "library" {
            required(
                component
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.kind == "file"),
                "executable component has no file artifact",
            )?;
        }
        // Services, clients and libraries make up the product; only a tool may
        // be unavailable on one of its targets.
        if role != "tool" {
            required(
                component.targets.len() == product_targets.len(),
                format!("{role} lacks a product target"),
            )?;
        }
        if role == "client" {
            required(
                component.requires.iter().any(|requirement| {
                    component_roles.get(requirement.component.as_str()) == Some(&"service")
                }),
                "client has no service requirement",
            )?;
        }

        for requirement in &component.requires {
            let target = components
                .get(requirement.component.as_str())
                .ok_or_else(|| Error("requirement references missing component".into()))?;
            required(
                component
                    .targets
                    .iter()
                    .all(|name| target.targets.contains(name)),
                "required component lacks a requiring component's target",
            )?;
            for interface in &requirement.interfaces {
                required(
                    target.interfaces.contains(interface),
                    "required interface is incompatible",
                )?;
            }
        }
    }

    required(
        !manifest.activation.required_contracts.is_empty(),
        "activation has no required contracts",
    )?;
    let mut required_contracts = HashSet::new();
    for requirement in &manifest.activation.required_contracts {
        required(
            required_contracts.insert((
                requirement.component.as_str(),
                requirement.contract.as_str(),
            )),
            "duplicate activation contract",
        )?;
        let component = components
            .get(requirement.component.as_str())
            .ok_or_else(|| Error("activation contract references missing component".into()))?;
        required(
            component
                .contracts
                .iter()
                .any(|contract| contract.id == requirement.contract),
            "activation references an unproved contract",
        )?;
    }

    required(
        !manifest.activation.required_interfaces.is_empty(),
        "activation has no required interfaces",
    )?;
    let mut required_interfaces = HashSet::new();
    for requirement in &manifest.activation.required_interfaces {
        required(
            required_interfaces.insert((
                requirement.component.as_str(),
                requirement.interface.as_str(),
                requirement.version,
            )),
            "duplicate activation interface",
        )?;
        let component = components
            .get(requirement.component.as_str())
            .ok_or_else(|| Error("activation interface references missing component".into()))?;
        required(
            component.interfaces.iter().any(|interface| {
                interface.id == requirement.interface && interface.version == requirement.version
            }),
            "activation references an unproved interface",
        )?;
    }

    Ok(())
}

fn required(condition: bool, message: impl Into<String>) -> Result<(), Error> {
    condition.then_some(()).ok_or_else(|| Error(message.into()))
}

fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::{parse_and_validate, report, version_report};
    use serde_json::Value;

    const CANONICAL: &str = include_str!("../../../../components/eon-v5.json");

    fn component_mut<'a>(manifest: &'a mut Value, id: &str) -> &'a mut Value {
        manifest["components"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|component| component["id"] == id)
            .unwrap()
    }

    fn rejected(expected: &str, manifest: &Value) {
        let input = serde_json::to_string(manifest).unwrap();
        let actual = parse_and_validate(&input).unwrap_err().to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn canonical_manifest_is_valid() {
        parse_and_validate(CANONICAL).unwrap();
    }

    #[test]
    fn removed_graph_fields_are_rejected() {
        let canonical: Value = serde_json::from_str(CANONICAL).unwrap();

        let mut path = canonical.clone();
        component_mut(&mut path, "orbit")["artifacts"][0]["path"] = "bin/other".into();
        assert!(parse_and_validate(&serde_json::to_string(&path).unwrap()).is_err());

        let mut launch = canonical.clone();
        component_mut(&mut launch, "orbit")["launch"] =
            serde_json::json!({ "artifact": "terminfo" });
        assert!(parse_and_validate(&serde_json::to_string(&launch).unwrap()).is_err());

        // A requirement names its target; the target alone carries the revision.
        let mut required_revision = canonical;
        component_mut(&mut required_revision, "venus")["requires"][0]["revision"] =
            Value::String("1".repeat(40));
        assert!(parse_and_validate(&serde_json::to_string(&required_revision).unwrap()).is_err());
    }

    #[test]
    fn version_report_contains_identity_without_resolved_paths() {
        // A tool may be unavailable on one of the product's targets.
        let mut manifest: Value = serde_json::from_str(CANONICAL).unwrap();
        component_mut(&mut manifest, "codexbar")["targets"] = serde_json::json!(["x86_64-linux"]);
        let input = serde_json::to_string(&manifest).unwrap();
        let targets = manifest["product"]["targets"].as_array().unwrap();
        assert_eq!(targets, &["x86_64-linux", "aarch64-darwin"]);

        let mut reports = Vec::new();
        for target in targets.iter().map(|target| target.as_str().unwrap()) {
            let report = report(&input, target).unwrap();
            let mut lines = report.lines();
            assert_eq!(
                lines.next().unwrap(),
                format!("{} {target}", manifest["product"]["id"].as_str().unwrap())
            );
            for component in manifest["components"].as_array().unwrap() {
                if component["targets"]
                    .as_array()
                    .unwrap()
                    .contains(&target.into())
                {
                    let mut identity = ["id", "version", "revision"]
                        .map(|key| component[key].as_str().unwrap_or("workspace"))
                        .to_vec();
                    identity.push(target);
                    assert_eq!(
                        lines.next().unwrap().split(' ').collect::<Vec<_>>(),
                        identity
                    );
                }
            }
            assert!(lines.next().is_none());
            reports.push(report);
        }
        assert!(reports[0].contains("\ncodexbar "));
        assert!(!reports[1].contains("\ncodexbar "));
        assert!(reports.contains(&version_report(&input).unwrap()));
        assert_eq!(
            report(&input, "aarch64-linux").unwrap_err().to_string(),
            "unsupported build target"
        );
    }

    #[test]
    fn invalid_graphs_are_rejected() {
        let canonical: Value = serde_json::from_str(CANONICAL).unwrap();

        let mut missing_component = canonical.clone();
        missing_component["components"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        rejected("role references missing component", &missing_component);

        let mut no_file_artifact = canonical.clone();
        component_mut(&mut no_file_artifact, "nushell")["artifacts"][0]["kind"] =
            "cargo-package".into();
        rejected(
            "executable component has no file artifact",
            &no_file_artifact,
        );

        let mut null_revision = canonical.clone();
        component_mut(&mut null_revision, "anima")["revision"] = Value::String("0".repeat(40));
        rejected("invalid component revision", &null_revision);

        let mut unpinned = canonical.clone();
        component_mut(&mut unpinned, "anima")
            .as_object_mut()
            .unwrap()
            .remove("revision");
        rejected("invalid component revision", &unpinned);

        let mut pinned_workspace = canonical.clone();
        component_mut(&mut pinned_workspace, "orbit")["revision"] = Value::String("1".repeat(40));
        rejected(
            "workspace component cannot carry a revision",
            &pinned_workspace,
        );

        for path in ["", "/orbit", "orbit/", "../orbit", "orbit/./src"] {
            let mut graph = canonical.clone();
            component_mut(&mut graph, "orbit")["source"]["path"] = Value::String(path.into());
            rejected("invalid workspace component path", &graph);
        }

        let mut bad_interface = canonical.clone();
        component_mut(&mut bad_interface, "venus")["requires"][0]["interfaces"][0]["version"] =
            Value::from(9);
        rejected("required interface is incompatible", &bad_interface);

        let mut resolved_path = canonical.clone();
        component_mut(&mut resolved_path, "helix")["project"] =
            Value::String("/nix/store/example/bin/hx".into());
        rejected(
            "Nix store paths are resolved inputs, not component identity",
            &resolved_path,
        );

        let mut nul_string = canonical.clone();
        component_mut(&mut nul_string, "helix")["project"] = Value::String("Helix\0".into());
        rejected("manifest string contains NUL", &nul_string);

        for url in ["https:///repo.git", "https://example.com/.git"] {
            let mut graph = canonical.clone();
            component_mut(&mut graph, "anima")["source"]["url"] = Value::String(url.into());
            rejected("component source must be an HTTPS Git repository", &graph);
        }

        for path in [r"\u002fnix\/store\/escaped", r"\u002fnix\/store"] {
            let escaped_path = CANONICAL.replacen(
                "\"project\": \"Venus\"",
                &format!("\"project\": \"{path}\""),
                1,
            );
            assert!(!escaped_path.contains("/nix/store/"));
            assert!(parse_and_validate(&escaped_path).is_err());
        }

        let mut no_targets = canonical.clone();
        no_targets["product"]["targets"] = serde_json::json!([]);
        rejected("product has no targets", &no_targets);

        let mut duplicate_target = canonical.clone();
        duplicate_target["product"]["targets"] =
            serde_json::json!(["x86_64-linux", "x86_64-linux"]);
        rejected("duplicate product target", &duplicate_target);

        for target in ["aarch64-linux", "x86_64-darwin", ""] {
            let mut graph = canonical.clone();
            graph["product"]["targets"][1] = Value::String(target.into());
            rejected("unsupported product target", &graph);
        }

        let mut no_component_targets = canonical.clone();
        component_mut(&mut no_component_targets, "helix")["targets"] = serde_json::json!([]);
        rejected("component has no targets", &no_component_targets);

        let mut duplicate_component_target = canonical.clone();
        component_mut(&mut duplicate_component_target, "helix")["targets"] =
            serde_json::json!(["x86_64-linux", "x86_64-linux"]);
        rejected("duplicate component target", &duplicate_component_target);

        for target in ["aarch64-linux", "aarch64-darwin"] {
            let mut graph = canonical.clone();
            graph["product"]["targets"] = serde_json::json!(["x86_64-linux"]);
            for component in graph["components"].as_array_mut().unwrap() {
                component["targets"] = serde_json::json!(["x86_64-linux"]);
            }
            component_mut(&mut graph, "helix")["targets"][0] = Value::String(target.into());
            rejected("component target is not a product target", &graph);
        }

        for (id, role) in [
            ("orbit", "service"),
            ("venus", "client"),
            ("eon-runtime", "library"),
        ] {
            let mut graph = canonical.clone();
            component_mut(&mut graph, id)["targets"] = serde_json::json!(["x86_64-linux"]);
            rejected(&format!("{role} lacks a product target"), &graph);
        }

        let mut unavailable_requirement = canonical.clone();
        let interfaces =
            serde_json::json!([{ "id": "quota", "version": 1, "proof": "1".repeat(40) }]);
        let codexbar = component_mut(&mut unavailable_requirement, "codexbar");
        codexbar["interfaces"] = interfaces.clone();
        codexbar["targets"] = serde_json::json!(["x86_64-linux"]);
        component_mut(&mut unavailable_requirement, "helix")["requires"] =
            serde_json::json!([{ "component": "codexbar", "interfaces": interfaces }]);
        rejected(
            "required component lacks a requiring component's target",
            &unavailable_requirement,
        );

        let mut mutable_revision = canonical.clone();
        component_mut(&mut mutable_revision, "yazi")["revision"] = Value::String("v26.5.6".into());
        rejected("invalid component revision", &mutable_revision);

        let duplicate = CANONICAL.replacen(
            "{\n  \"schema\": 5,",
            "{\n  \"schema\": 5,\n  \"schema\": 5,",
            1,
        );
        assert!(parse_and_validate(&duplicate).is_err());
    }
}
