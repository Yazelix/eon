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
    target: String,
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
    revision: String,
    target: String,
    source: Source,
    artifacts: Vec<Artifact>,
    contracts: Vec<Contract>,
    interfaces: Vec<Interface>,
    requires: Vec<Requirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    kind: String,
    url: String,
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
    revision: String,
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

pub fn version_report(input: &str) -> Result<String, Error> {
    let manifest = parse(input)?;
    validate(&manifest)?;
    let mut lines = vec![format!(
        "{} {}",
        manifest.product.id, manifest.product.target
    )];
    lines.extend(manifest.components.iter().map(|component| {
        format!(
            "{} {} {} {}",
            component.id, component.version, component.revision, component.target
        )
    }));
    Ok(lines.join("\n"))
}

pub fn component_revision(input: &str, id: &str) -> Result<String, Error> {
    let manifest = parse(input)?;
    validate(&manifest)?;
    manifest
        .components
        .into_iter()
        .find(|component| component.id == id)
        .map(|component| component.revision)
        .ok_or_else(|| Error(format!("manifest has no {id} component")))
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
    required(manifest.schema == 3, "unsupported manifest schema")?;
    required(token(&manifest.product.id), "invalid product id")?;
    required(token(&manifest.product.target), "invalid product target")?;
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
        required(revision(&component.revision), "invalid component revision")?;
        required(
            component.target == manifest.product.target,
            "component target differs from product target",
        )?;
        required(
            component.source.kind == "git"
                && component
                    .source
                    .url
                    .strip_prefix("https://")
                    .and_then(|source| source.split_once('/'))
                    .is_some_and(|(host, path)| {
                        !host.is_empty()
                            && path.rsplit('/').next().is_some_and(|repository| {
                                repository != ".git" && repository.ends_with(".git")
                            })
                    }),
            "component source must be an HTTPS Git repository",
        )?;
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
            required(revision(&requirement.revision), "invalid required revision")?;
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
                target.revision == requirement.revision,
                "required component revision is incompatible",
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
    use super::{component_revision, parse_and_validate, version_report};
    use serde_json::Value;

    const CANONICAL: &str = include_str!("../../../components/eon-alpha-v3.json");

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

        let mut launch = canonical;
        component_mut(&mut launch, "orbit")["launch"] =
            serde_json::json!({ "artifact": "terminfo" });
        assert!(parse_and_validate(&serde_json::to_string(&launch).unwrap()).is_err());
    }

    #[test]
    fn version_report_contains_identity_without_resolved_paths() {
        let manifest: Value = serde_json::from_str(CANONICAL).unwrap();
        let report = version_report(CANONICAL).unwrap();
        let mut lines = report.lines();
        assert_eq!(
            lines.next().unwrap().split(' ').collect::<Vec<_>>(),
            ["id", "target"].map(|key| manifest["product"][key].as_str().unwrap())
        );
        for component in manifest["components"].as_array().unwrap() {
            assert_eq!(
                lines.next().unwrap().split(' ').collect::<Vec<_>>(),
                ["id", "version", "revision", "target"].map(|key| component[key].as_str().unwrap())
            );
            assert_eq!(
                component_revision(CANONICAL, component["id"].as_str().unwrap()).unwrap(),
                component["revision"].as_str().unwrap()
            );
        }
        assert!(lines.next().is_none());
        assert!(component_revision(CANONICAL, "missing").is_err());
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

        let mut incompatible = canonical.clone();
        component_mut(&mut incompatible, "venus")["requires"][0]["revision"] =
            Value::String("1".repeat(40));
        rejected("required component revision is incompatible", &incompatible);

        let mut null_revision = canonical.clone();
        component_mut(&mut null_revision, "orbit")["revision"] = Value::String("0".repeat(40));
        rejected("invalid component revision", &null_revision);

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
            component_mut(&mut graph, "orbit")["source"]["url"] = Value::String(url.into());
            rejected("component source must be an HTTPS Git repository", &graph);
        }

        for path in [r"\u002fnix\/store\/escaped", r"\u002fnix\/store"] {
            let escaped_path = CANONICAL.replacen(
                "\"project\": \"Eon Desktop\"",
                &format!("\"project\": \"{path}\""),
                1,
            );
            assert!(!escaped_path.contains("/nix/store/"));
            assert!(parse_and_validate(&escaped_path).is_err());
        }

        let mut mutable_revision = canonical.clone();
        component_mut(&mut mutable_revision, "yazi")["revision"] = Value::String("v26.5.6".into());
        rejected("invalid component revision", &mutable_revision);

        let duplicate = CANONICAL.replacen(
            "{\n  \"schema\": 3,",
            "{\n  \"schema\": 3,\n  \"schema\": 3,",
            1,
        );
        assert!(parse_and_validate(&duplicate).is_err());
    }
}
