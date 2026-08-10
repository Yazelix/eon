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
    launch: Option<Launch>,
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
    path: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    artifact: String,
    #[serde(rename = "arguments")]
    _arguments: Vec<String>,
    inputs: Vec<String>,
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
    required(manifest.schema == 1, "unsupported manifest schema")?;
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
            required(relative(&artifact.path), "artifact path must be relative")?;
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
        match (&component.launch, role) {
            (None, "library") => {}
            (Some(_), "library") => return Err(Error("library cannot have a launch plan".into())),
            (None, _) => return Err(Error("executable component has no launch plan".into())),
            (Some(launch), _) => {
                required(
                    component
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.id == launch.artifact && artifact.kind == "file"),
                    "launch artifact is not a declared file",
                )?;
                required(!launch.inputs.is_empty(), "launch plan has no inputs")?;
                let mut inputs = HashSet::new();
                for input in &launch.inputs {
                    required(
                        token(input) && inputs.insert(input.as_str()),
                        "invalid or duplicate launch input",
                    )?;
                }
            }
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

fn relative(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."))
}

#[cfg(test)]
mod tests {
    use super::{parse_and_validate, version_report};
    use serde_json::Value;

    const CANONICAL: &str = include_str!("../../../components/eon-alpha-v1.json");

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
    fn version_report_contains_identity_without_resolved_paths() {
        let report = version_report(CANONICAL).unwrap();

        assert_eq!(
            report,
            "eon-alpha x86_64-linux\n\
orbit 0.1.0 64db581445bafca1a08a6530f8e44f9c1edbc169 x86_64-linux\n\
venus 0.1.0 e7bda96822274727faacb51731ae19181295e1cd x86_64-linux\n\
nushell 0.113.1 7b7df4aa68e957cf38b9d8157c35fa7523f44a6d x86_64-linux\n\
bash 5.3p9 b8c60bc9ca365f8261fa97900b6fa939f6ebc303 x86_64-linux\n\
zsh 5.9.1 0e0d4ea11731c47f57bad042fbe75e3979d8a1d2 x86_64-linux\n\
fish 4.7.1 efb0223da10367031b7c887a3e40eccdf9bf7b06 x86_64-linux\n\
starship 1.25.1 8758daa7767d4e73874330b1e262fca66a7ffd30 x86_64-linux\n\
zoxide 0.9.9 9cdc6aa3740b4d8a9d62406c99e84c5de49645e9 x86_64-linux\n\
atuin 18.16.1 671f96b60dac49d1d2de73cc0812986a5e22ce7b x86_64-linux\n\
carapace 1.6.3 e4ed2a5ae661848b228224ad7edb20ea678d33d4 x86_64-linux\n\
helix 25.7.1 7e6cd307d00783c16ad4cff99ed71936d34f6572 x86_64-linux\n\
yazi 26.5.6 aa526434f00bb44e2e902d9a4ac5f810da1018b9 x86_64-linux\n\
lazygit 0.62.2 009c8975beb322f9374789476ac65dfa02321fce x86_64-linux\n\
ratconfig 6.0.0 e6ec2ebfe84b2358186410680cbcaf0564eb59a2 x86_64-linux"
        );
        assert!(!report.contains("/nix/store"));
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
        component_mut(&mut resolved_path, "helix")["artifacts"][0]["path"] =
            Value::String("/nix/store/example/bin/hx".into());
        rejected(
            "Nix store paths are resolved inputs, not component identity",
            &resolved_path,
        );

        let mut nul_path = canonical.clone();
        component_mut(&mut nul_path, "helix")["artifacts"][0]["path"] =
            Value::String("bin/\0hx".into());
        rejected("manifest string contains NUL", &nul_path);

        for url in ["https:///repo.git", "https://example.com/.git"] {
            let mut graph = canonical.clone();
            component_mut(&mut graph, "orbit")["source"]["url"] = Value::String(url.into());
            rejected("component source must be an HTTPS Git repository", &graph);
        }

        for path in [r"\u002fnix\/store\/escaped", r"\u002fnix\/store"] {
            let escaped_path = CANONICAL.replacen(
                "\"arguments\": [],",
                &format!("\"arguments\": [\"{path}\"],"),
                1,
            );
            assert!(!escaped_path.contains("/nix/store/"));
            assert!(parse_and_validate(&escaped_path).is_err());
        }

        let mut mutable_revision = canonical.clone();
        component_mut(&mut mutable_revision, "yazi")["revision"] = Value::String("v26.5.6".into());
        rejected("invalid component revision", &mutable_revision);

        let duplicate = CANONICAL.replacen(
            "{\n  \"schema\": 1,",
            "{\n  \"schema\": 1,\n  \"schema\": 1,",
            1,
        );
        assert!(parse_and_validate(&duplicate).is_err());
    }
}
