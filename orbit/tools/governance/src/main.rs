use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const CONTRACTS: &str = "orbit/docs/CONTRACTS.md";
const CRATES: &str = "orbit/docs/CRATES.md";
const BEADS: &str = ".beads/issues.jsonl";
const CONTRACT_STATUSES: &[&str] = &["Planned", "Partially proved", "Proved", "Retired"];
const DECISION_STATUSES: &[&str] = &["Selected", "Planned", "Candidate", "Deferred"];

#[derive(Debug, Deserialize)]
struct Issue {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    design: String,
    #[serde(default)]
    acceptance_criteria: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    close_reason: String,
}

fn read_text(root: &Path, relative: &str, errors: &mut Vec<String>) -> String {
    match fs::read_to_string(root.join(relative)) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("{relative}: {error}"));
            String::new()
        }
    }
}

fn contract_id(id: &str) -> Option<&str> {
    let number = id.strip_prefix("ORB-C")?;
    if !number.is_empty()
        && !number.starts_with('0')
        && number.bytes().all(|byte| byte.is_ascii_digit())
    {
        Some(id)
    } else {
        None
    }
}

#[derive(Debug)]
struct ContractSection {
    id: String,
    title: String,
    fields: BTreeMap<String, String>,
}

fn parse_labeled_field(
    line: &str,
    relative: &str,
    section: &str,
    fields: &mut BTreeMap<String, String>,
    current_field: &mut Option<String>,
    errors: &mut Vec<String>,
) {
    let trimmed = line.trim_start();
    if let Some(field) = trimmed.strip_prefix("- **")
        && let Some((name, value)) = field.split_once(":**")
    {
        let name = name.to_owned();
        if fields
            .insert(name.clone(), value.trim().to_owned())
            .is_some()
        {
            errors.push(format!("{relative}: {section} has duplicate {name} field"));
        }
        *current_field = Some(name);
    } else if !trimmed.is_empty()
        && let Some(name) = current_field.as_ref()
    {
        let value = fields.get_mut(name).expect("current field exists");
        if !value.is_empty() {
            value.push(' ');
        }
        value.push_str(trimmed);
    }
}

fn contract_sections(text: &str, errors: &mut Vec<String>) -> Vec<ContractSection> {
    let mut sections = Vec::new();
    let mut current: Option<ContractSection> = None;
    let mut current_field: Option<String> = None;

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current_field = None;
            if heading.starts_with("ORB-") {
                let (id, title) = heading.split_once(" — ").unwrap_or((heading, ""));
                current = Some(ContractSection {
                    id: id.to_owned(),
                    title: title.to_owned(),
                    fields: BTreeMap::new(),
                });
            }
            continue;
        }

        let Some(section) = current.as_mut() else {
            continue;
        };
        if !line.trim_start().starts_with("## ") {
            parse_labeled_field(
                line,
                CONTRACTS,
                &section.id,
                &mut section.fields,
                &mut current_field,
                errors,
            );
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    if sections.is_empty() {
        errors.push(format!("{CONTRACTS}: no ORB-C* contract sections"));
    }
    sections
}

fn lowercase_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn check_contracts(
    text: &str,
    known_beads: &BTreeSet<String>,
    errors: &mut Vec<String>,
) -> BTreeSet<String> {
    const REQUIRED_FIELDS: &[&str] = &[
        "Status",
        "Consumer",
        "Trigger",
        "Result",
        "Important failures",
        "Owner",
        "Boundary",
        "Proof",
    ];
    const ALLOWED_FIELDS: &[&str] = &[
        "Status",
        "Consumer",
        "Trigger",
        "Result",
        "Important failures",
        "Owner",
        "Consumes",
        "Boundary",
        "Proof",
        "Evidence",
        "Environment",
        "Open proof",
    ];

    let sections = contract_sections(text, errors);
    let mut known = BTreeSet::new();
    for section in sections {
        let Some(id) = contract_id(&section.id) else {
            errors.push(format!("{CONTRACTS}: malformed contract ID {}", section.id));
            continue;
        };
        if !known.insert(id.to_owned()) {
            errors.push(format!("{CONTRACTS}: duplicate contract ID {id}"));
        }
        if section.title.is_empty() {
            errors.push(format!("{CONTRACTS}: {id} requires a title"));
        }
        for field in REQUIRED_FIELDS {
            if section
                .fields
                .get(*field)
                .is_none_or(|value| value.is_empty())
            {
                errors.push(format!(
                    "{CONTRACTS}: {id} requires a nonempty {field} field"
                ));
            }
        }
        for field in section.fields.keys() {
            if !ALLOWED_FIELDS.contains(&field.as_str()) {
                errors.push(format!("{CONTRACTS}: {id} has unknown {field} field"));
            }
        }
        let status = section
            .fields
            .get("Status")
            .map(String::as_str)
            .unwrap_or("");
        if !CONTRACT_STATUSES.contains(&status) {
            errors.push(format!(
                "{CONTRACTS}: {id} has invalid contract status {status}"
            ));
        }
        if status == "Proved" {
            // As in Eon's index: a proof names its full commit, or the owning
            // Bead whose commit is the proof's revision.
            let proof = section
                .fields
                .get("Proof")
                .map(String::as_str)
                .unwrap_or("");
            if !has_full_commit(proof) && !cites_known_bead(proof, known_beads) {
                errors.push(format!(
                    "{CONTRACTS}: Proved {id} requires a full proof commit or its owning Bead"
                ));
            }
            if section
                .fields
                .get("Evidence")
                .is_none_or(|value| matches!(value.as_str(), "" | "—" | "-"))
            {
                errors.push(format!(
                    "{CONTRACTS}: Proved {id} requires a named check or evidence"
                ));
            }
        }
    }
    known
}

fn contract_references(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut references = BTreeSet::new();
    for (start, _) in text.match_indices("ORB-C") {
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        let mut end = start + "ORB-C".len();
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == start + "ORB-C".len()
            || (end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_'))
        {
            continue;
        }
        references.insert(text[start..end].to_owned());
    }
    references
}

fn load_issues(text: &str, errors: &mut Vec<String>) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Issue>(line) {
            Ok(issue) => issues.push(issue),
            Err(error) => errors.push(format!(
                "{BEADS}:{}: invalid issue JSON: {error}",
                number + 1
            )),
        }
    }
    issues
}

fn issue_text(issue: &Issue) -> String {
    [
        issue.description.as_str(),
        issue.design.as_str(),
        issue.acceptance_criteria.as_str(),
        issue.notes.as_str(),
        issue.close_reason.as_str(),
    ]
    .join("\n")
}

fn check_issue_references(
    issues: &[Issue],
    known_contracts: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    for issue in issues.iter().filter(|issue| issue.status != "deferred") {
        for unknown in contract_references(&issue_text(issue)).difference(known_contracts) {
            errors.push(format!(
                "{}: unknown contract reference {unknown}",
                issue.id
            ));
        }
    }
}

fn has_exact_version(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_digit() && character != '.')
        .any(|candidate| {
            let parts = candidate.split('.').collect::<Vec<_>>();
            matches!(parts.len(), 2 | 3)
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

fn has_full_commit(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_hexdigit())
        .any(|candidate| lowercase_hex(candidate, 40))
}

#[derive(Debug)]
struct DecisionSection {
    boundary: String,
    fields: BTreeMap<String, String>,
}

fn decision_sections(text: &str, errors: &mut Vec<String>) -> Vec<DecisionSection> {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(position) = lines
        .iter()
        .position(|line| *line == "## Current decisions")
    else {
        errors.push(format!("{CRATES}: missing ## Current decisions section"));
        return Vec::new();
    };
    let mut sections = Vec::new();
    let mut current: Option<DecisionSection> = None;
    let mut current_field: Option<String> = None;

    for line in &lines[position + 1..] {
        if line.starts_with("## ") {
            break;
        }
        if let Some(boundary) = line.strip_prefix("### ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(DecisionSection {
                boundary: boundary.to_owned(),
                fields: BTreeMap::new(),
            });
            current_field = None;
            continue;
        }

        let Some(section) = current.as_mut() else {
            continue;
        };
        parse_labeled_field(
            line,
            CRATES,
            &section.boundary,
            &mut section.fields,
            &mut current_field,
            errors,
        );
    }
    if let Some(section) = current {
        sections.push(section);
    }
    if sections.is_empty() {
        errors.push(format!("{CRATES}: no decision sections"));
    }
    sections
}

fn cites_known_bead(text: &str, known_beads: &BTreeSet<String>) -> bool {
    text.split(|character: char| {
        !character.is_ascii_lowercase()
            && !character.is_ascii_digit()
            && character != '-'
            && character != '.'
    })
    .any(|candidate| known_beads.contains(candidate.trim_end_matches('.')))
}

fn check_crates(text: &str, known_beads: &BTreeSet<String>, errors: &mut Vec<String>) {
    const REQUIRED_FIELDS: &[&str] = &[
        "Selected shape",
        "Status",
        "Credible alternatives",
        "Why",
        "Evidence",
    ];

    let sections = decision_sections(text, errors);
    let mut boundaries = BTreeSet::new();
    for section in sections {
        let boundary = &section.boundary;
        if boundary.is_empty() {
            errors.push(format!("{CRATES}: decision requires a boundary heading"));
        } else if !boundaries.insert(boundary.clone()) {
            errors.push(format!("{CRATES}: duplicate decision boundary {boundary}"));
        }
        for field in REQUIRED_FIELDS {
            if section
                .fields
                .get(*field)
                .is_none_or(|value| value.is_empty())
            {
                errors.push(format!(
                    "{CRATES}: {boundary} requires a nonempty {field} field"
                ));
            }
        }
        for field in section.fields.keys() {
            if !REQUIRED_FIELDS.contains(&field.as_str()) {
                errors.push(format!("{CRATES}: {boundary} has unknown {field} field"));
            }
        }
        let status = section
            .fields
            .get("Status")
            .map(String::as_str)
            .unwrap_or("");
        let category = status.split_whitespace().next().unwrap_or_default();
        if !DECISION_STATUSES.contains(&category) {
            errors.push(format!(
                "{CRATES}: {boundary} has invalid decision status {status}"
            ));
            continue;
        }
        if category != "Selected" {
            continue;
        }
        let selected = section
            .fields
            .get("Selected shape")
            .map_or("", String::as_str);
        if !has_exact_version(selected) && !has_full_commit(selected) {
            errors.push(format!(
                "{CRATES}: selected {boundary} decision lacks an exact version or commit"
            ));
        }
        let alternatives = section
            .fields
            .get("Credible alternatives")
            .map_or("", String::as_str);
        if matches!(alternatives, "" | "—" | "-" | "None") {
            errors.push(format!(
                "{CRATES}: selected {boundary} decision lacks credible alternatives"
            ));
        }
        let evidence = section.fields.get("Evidence").map_or("", String::as_str);
        if !cites_known_bead(evidence, known_beads) {
            errors.push(format!(
                "{CRATES}: selected {boundary} decision lacks an existing evidence Bead"
            ));
        }
    }
}

fn check_unknown_references(
    relative: &str,
    text: &str,
    known_contracts: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    for unknown in contract_references(text).difference(known_contracts) {
        errors.push(format!("{relative}: unknown contract reference {unknown}"));
    }
}

fn check_repository(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let beads = read_text(root, BEADS, &mut errors);
    let issues = load_issues(&beads, &mut errors);
    let known_beads = issues.iter().map(|issue| issue.id.clone()).collect();
    let contracts = read_text(root, CONTRACTS, &mut errors);
    let known_contracts = check_contracts(&contracts, &known_beads, &mut errors);
    check_issue_references(&issues, &known_contracts, &mut errors);
    let crates = read_text(root, CRATES, &mut errors);
    check_crates(&crates, &known_beads, &mut errors);
    check_unknown_references(CONTRACTS, &contracts, &known_contracts, &mut errors);
    check_unknown_references(CRATES, &crates, &known_contracts, &mut errors);
    errors
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let root = match (arguments.next(), arguments.next()) {
        (Some(root), None) => PathBuf::from(root),
        (None, None) => match std::env::current_dir() {
            Ok(root) => root,
            Err(error) => {
                eprintln!("error: cannot determine current directory: {error}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("usage: orbit-governance [ROOT]");
            return ExitCode::FAILURE;
        }
    };
    let errors = check_repository(&root);
    if errors.is_empty() {
        println!("governance: ok");
        ExitCode::SUCCESS
    } else {
        for error in errors {
            eprintln!("error: {error}");
        }
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    static NEXT_REPOSITORY: AtomicU64 = AtomicU64::new(0);

    struct TestRepository {
        root: std::path::PathBuf,
        issue: Value,
    }

    impl TestRepository {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "orbit-governance-{}-{}",
                std::process::id(),
                NEXT_REPOSITORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("create fixture root");
            let issue = json!({
                "id": "eon-proof.1",
                "status": "closed",
                "issue_type": "task",
                "description": "This docs-only task preserves ORB-C1."
            });
            let repository = Self { root, issue };
            repository.write_fixture();
            repository
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create fixture directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        fn write_fixture(&self) {
            self.write(
                CONTRACTS,
                &format!(
                    "# Contracts\n\n## ORB-C1 — Durable session\n\n\
                     - **Status:** Proved\n\
                     - **Consumer:** One local client.\n\
                     - **Trigger:** The client disconnects and later reconnects.\n\
                     - **Result:** The Session remains live.\n\
                     - **Important failures:** Connection failure is explicit.\n\
                     - **Owner:** Orbit.\n\
                     - **Boundary:** Restart persistence is excluded.\n\
                     - **Proof:** `{COMMIT}`\n\
                       - **Evidence:** canonical contract test\n\n\
                     ## ORB-C2 — Bead-named proof\n\n\
                     - **Status:** Proved\n\
                     - **Consumer:** One local client.\n\
                     - **Trigger:** The client attaches.\n\
                     - **Result:** It sees the Session.\n\
                     - **Important failures:** Attachment failure is explicit.\n\
                     - **Owner:** Orbit.\n\
                     - **Boundary:** One client.\n\
                     - **Proof:** The `eon-proof.1` commit\n\
                       - **Evidence:** canonical contract test\n"
                ),
            );
            self.write(
                CRATES,
                "# Crates\n\n## Current decisions\n\n\
                 ### Terminal\n\n\
                 - **Selected shape:** libghostty-vt 0.2.1\n\
                 - **Status:** Selected for proof\n\
                 - **Credible alternatives:** owned parser\n\
                 - **Why:** Sole authority\n\
                 - **Evidence:** eon-proof.1.\n\n\
                 ### Renderer\n\n\
                 - **Selected shape:** Undecided\n\
                 - **Status:** Planned\n\
                 - **Credible alternatives:** wgpu or owned code\n\
                 - **Why:** Deferred\n\
                 - **Evidence:** future Bead\n",
            );
            self.write_issue();
        }

        fn write_issue(&self) {
            self.write(
                BEADS,
                &(serde_json::to_string(&self.issue).expect("serialize issue") + "\n"),
            );
        }

        fn errors(&self) -> Vec<String> {
            check_repository(&self.root)
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn valid_repository_accepts_docs_only_task_without_gate_markers() {
        let repository = TestRepository::new();
        let issue = serde_json::to_string(&repository.issue).expect("serialize issue");
        repository.write(BEADS, &format!("{issue}\n{issue}\n"));

        assert!(repository.errors().is_empty());
    }

    #[test]
    fn deferred_proposals_and_archival_comments_are_not_current_contract_claims() {
        let mut repository = TestRepository::new();
        repository.issue["status"] = json!("deferred");
        repository.issue["description"] = json!("Proposed ORB-C9 requires approval.");
        repository.issue["comments"] = json!([{"text": "Historical ORB-C8 proposal."}]);
        repository.write_issue();

        assert!(repository.errors().is_empty());

        repository.issue["status"] = json!("open");
        repository.issue["description"] = json!("Preserve ORB-C1.");
        repository.write_issue();

        assert!(repository.errors().is_empty());

        repository.issue["description"] = json!("Proposed ORB-C9 requires approval.");
        repository.write_issue();

        assert!(
            repository
                .errors()
                .join("\n")
                .contains("unknown contract reference ORB-C9")
        );
    }

    #[test]
    fn invalid_metadata_failures_are_reported_together() {
        let mut repository = TestRepository::new();
        repository.write(
            CONTRACTS,
            &format!(
                "# Contracts\n\n\
                 ## ORB-C1 — One\n\n\
                 - **Status:** Proved\n\
                 - **Consumer:** Client.\n\
                 - **Trigger:** Input.\n\
                 - **Result:** Preserve ORB-C8.\n\
                 - **Important failures:** Explicit.\n\
                 - **Owner:** Orbit.\n\
                 - **Owner:** Duplicate owner.\n\
                 - **Boundary:** None.\n\
                 - **Proof:** `{COMMIT}`\n\
                   - **Evidence:** canonical test\n\n\
                 ## ORB-C1 — Duplicate\n\n\
                 - **Status:** Done\n\
                 - **Consumer:** Client.\n\
                 - **Trigger:** Input.\n\
                 - **Result:** Duplicate.\n\
                 - **Important failures:** Explicit.\n\
                 - **Owner:** Orbit.\n\
                 - **Boundary:** None.\n\
                 - **Proof:** `{COMMIT}`\n\
                   - **Evidence:** canonical test\n\n\
                 ## ORB-X — Malformed\n\n\
                 - **Status:** Planned\n\
                 - **Consumer:** Client.\n\
                 - **Trigger:** Input.\n\
                 - **Result:** Planned.\n\
                 - **Important failures:** Explicit.\n\
                 - **Owner:** Orbit.\n\
                 - **Boundary:** Open.\n\
                 - **Proof:** None.\n\n\
                 ## ORB-C2 — Bad proof\n\n\
                 - **Status:** Proved\n\
                 - **Consumer:** Client.\n\
                 - **Trigger:** Input.\n\
                 - **Result:** Bad proof.\n\
                 - **Important failures:** Explicit.\n\
                 - **Boundary:** None.\n\
                 - **Proof:** `edge`\n\
                   - **Evidence:** —\n\
                 - **Maybe:** typo\n"
            ),
        );
        repository.issue["description"] = json!("Preserve ORB-C1 and unknown ORB-C9.");
        repository.write(
            BEADS,
            &(serde_json::to_string(&repository.issue).expect("serialize issue") + "\n{\n"),
        );
        repository.write(
            CRATES,
            "# Crates\n\n## Current decisions\n\n\
             ### Runtime\n\n\
             - **Selected shape:** Owned loop\n\
             - **Status:** Selected\n\
             - **Credible alternatives:** —\n\
             - **Why:** Small; preserves ORB-C7\n\
             - **Evidence:** future work\n\n\
             ### Renderer\n\n\
             - **Selected shape:** Maybe\n\
             - **Status:** Chosen\n\
             - **Credible alternatives:** owned code\n\
             - **Evidence:** eon-proof.1\n\
             - **Unexpected:** no\n",
        );

        let errors = repository.errors().join("\n");

        assert!(errors.contains("duplicate contract ID ORB-C1"));
        assert!(errors.contains("ORB-C1 has duplicate Owner field"));
        assert!(errors.contains("malformed contract ID ORB-X"));
        assert!(errors.contains("invalid contract status Done"));
        assert!(errors.contains("ORB-C2 requires a nonempty Owner field"));
        assert!(errors.contains("ORB-C2 has unknown Maybe field"));
        assert!(errors.contains("ORB-C2 requires a full proof commit or its owning Bead"));
        assert!(errors.contains("ORB-C2 requires a named check or evidence"));
        assert!(errors.contains("unknown contract reference ORB-C9"));
        assert!(errors.contains("orbit/docs/CONTRACTS.md: unknown contract reference ORB-C8"));
        assert!(errors.contains("orbit/docs/CRATES.md: unknown contract reference ORB-C7"));
        assert!(errors.contains("invalid issue JSON"));
        assert!(errors.contains("Renderer requires a nonempty Why field"));
        assert!(errors.contains("Renderer has unknown Unexpected field"));
        assert!(errors.contains("selected Runtime decision lacks an exact version or commit"));
        assert!(errors.contains("selected Runtime decision lacks credible alternatives"));
        assert!(errors.contains("selected Runtime decision lacks an existing evidence Bead"));
        assert!(errors.contains("Renderer has invalid decision status Chosen"));
    }
}

#[cfg(test)]
mod dependency_direction {
    use serde::Deserialize;
    use std::{collections::BTreeSet, fs, path::PathBuf, process::Command};

    const PROTOCOLS: &[&str] = &["orbit-protocol", "eon-workspace-protocol"];
    const ALLOWED_WORKSPACE_DEPENDENCIES: &[(&str, &[&str])] = &[
        ("orbit-protocol", &[]),
        ("eon-workspace-protocol", &[]),
        ("yazelix-orbit", &["orbit-protocol"]),
        ("orbit-governance", &[]),
        ("yazelix-venus", PROTOCOLS),
        ("eon-runtime", PROTOCOLS),
        (
            "eon-manifest",
            &["eon-runtime", "orbit-protocol", "eon-workspace-protocol"],
        ),
        (
            "eon",
            &[
                "eon-runtime",
                "eon-manifest",
                "orbit-protocol",
                "eon-workspace-protocol",
            ],
        ),
    ];
    const TERMINAL_ENGINE: &str = "libghostty-vt";
    const TERMINAL_ENGINE_FAMILY: &str = "libghostty";
    const TERMINAL_ENGINE_OWNER: &str = "yazelix-orbit";

    #[derive(Deserialize)]
    struct Metadata {
        packages: Vec<Package>,
    }

    #[derive(Deserialize)]
    struct Package {
        name: String,
        manifest_path: PathBuf,
        dependencies: Vec<Dependency>,
    }

    #[derive(Deserialize)]
    struct Dependency {
        name: String,
    }

    fn workspace_packages() -> Vec<Package> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run cargo metadata");
        assert!(
            output.status.success(),
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Metadata>(&output.stdout)
            .expect("parse cargo metadata")
            .packages
    }

    fn inherits_workspace_settings(manifest: &str) -> bool {
        manifest.lines().any(|line| {
            let setting = line.split('#').next().unwrap_or_default();
            setting
                .split_whitespace()
                .collect::<String>()
                .contains("workspace=true")
        })
    }

    fn violations(packages: &[Package]) -> BTreeSet<String> {
        let members = packages
            .iter()
            .map(|package| package.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut errors = BTreeSet::new();
        for package in packages {
            let Some((_, allowed)) = ALLOWED_WORKSPACE_DEPENDENCIES
                .iter()
                .find(|(name, _)| *name == package.name)
            else {
                errors.insert(format!(
                    "workspace member {} has no dependency-direction rule",
                    package.name
                ));
                continue;
            };
            for dependency in &package.dependencies {
                let edge = format!("{} -> {}", package.name, dependency.name);
                if dependency.name.starts_with(TERMINAL_ENGINE_FAMILY)
                    && package.name != TERMINAL_ENGINE_OWNER
                {
                    errors.insert(format!(
                        "forbidden dependency {edge}: only {TERMINAL_ENGINE_OWNER} may depend on {TERMINAL_ENGINE_FAMILY} crates"
                    ));
                }
                if members.contains(dependency.name.as_str())
                    && !allowed.contains(&dependency.name.as_str())
                {
                    errors.insert(format!("forbidden workspace dependency {edge}"));
                }
            }
            if package.name == TERMINAL_ENGINE_OWNER
                && !package
                    .dependencies
                    .iter()
                    .any(|dependency| dependency.name == TERMINAL_ENGINE)
            {
                errors.insert(format!(
                    "{TERMINAL_ENGINE_OWNER} no longer depends on {TERMINAL_ENGINE}; update the terminal-engine rule"
                ));
            }
            if PROTOCOLS.contains(&package.name.as_str()) {
                let manifest =
                    fs::read_to_string(&package.manifest_path).expect("read protocol manifest");
                if inherits_workspace_settings(&manifest) {
                    errors.insert(format!(
                        "{} inherits workspace settings in {}",
                        package.name,
                        package.manifest_path.display()
                    ));
                }
            }
        }
        errors
    }

    #[test]
    fn workspace_crates_keep_their_dependency_direction() {
        let errors = violations(&workspace_packages());

        assert!(
            errors.is_empty(),
            "dependency direction violations:\n{}",
            errors.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
}
