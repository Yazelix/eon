//! Reviewed bootstrap images: the only images a remote run may start from.
//!
//! An image enters this table only after its own qualification (credential audit,
//! capped build, restored clone). It is code, not configuration, so an operator
//! setting cannot route builds to an image nobody qualified.
use super::super::memory::{Limit, Machine};
use crate::config::BoatConfig;
use serde_json::Value;
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::slots) struct Image {
    pub name: &'static str,
    pub snapshot_id: &'static str,
    pub source_vm: &'static str,
    pub environment: &'static str,
    pub environment_version: u64,
    pub machine: Machine,
    /// SHA-256 of the recipe manifest, held on the guest in [`MARKER`].
    pub recipe_sha256: &'static str,
}

/// eon-klt8h qualified the large image only; the stock default image stays unqualified.
const QUALIFIED: &[Image] = &[Image {
    name: "eon-builds-nix-2-34-7-large-klt8h-v1",
    snapshot_id: "9c882067-a76c-4580-9a53-c90ad574441e",
    source_vm: "bx_vfxzz24a",
    environment: "eon-builds",
    environment_version: 2,
    machine: Machine::Large,
    recipe_sha256: "7a5da4f0f0a1bf9de41b47695ae48f94ed115b7145cba5b941bcee2019dd708f",
}];

/// Guest check of the recipe marker: exit 0 verified, 94 absent or unreadable, 95 different.
pub(super) const MARKER: &str = include_str!("image.sh");

/// The smallest qualified image whose machine ceiling covers `limit`, or the capacity reason.
pub(in crate::slots) fn select(
    limit: Limit,
    config: &BoatConfig,
) -> Result<&'static Image, &'static str> {
    select_from(limit.cap_gib, config, QUALIFIED)
}

fn select_from(
    cap_gib: u8,
    config: &BoatConfig,
    table: &'static [Image],
) -> Result<&'static Image, &'static str> {
    let covering = [Machine::Default, Machine::Large]
        .into_iter()
        .filter(|machine| {
            cap_gib
                <= match machine {
                    Machine::Default => 6,
                    Machine::Large => config.large_max_memory_gib,
                }
        });
    let mut any = false;
    for machine in covering {
        any = true;
        if let Some(image) = table.iter().find(|image| image.machine == machine) {
            return Ok(image);
        }
    }
    Err(if any {
        "image-unpinned"
    } else {
        "unqualified-ceiling"
    })
}

/// Why a provider observation does not establish the pinned image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The provider or guest could not show the image either way.
    Unverified(&'static str),
    /// The provider or guest showed something other than the pinned image.
    Mismatch(&'static str),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unverified(check) => write!(f, "boat-image-unverified check={check}"),
            Self::Mismatch(check) => write!(f, "boat-image-mismatch check={check}"),
        }
    }
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        refusal.to_string()
    }
}

/// Check `boat snapshots` output for exactly one ready row that is the pinned image.
pub(super) fn verify(list: &Value, image: &Image) -> Result<(), Refusal> {
    use Refusal::{Mismatch, Unverified};
    let named = list["named"]
        .as_array()
        .ok_or(Unverified("snapshot-unreadable"))?;
    let mut rows = named.iter().filter(|row| row["name"] == image.name);
    let row = rows.next().ok_or(Unverified("snapshot-absent"))?;
    if rows.next().is_some() {
        return Err(Mismatch("snapshot-duplicate"));
    }
    for (field, expected, check) in [
        ("snapshotId", image.snapshot_id, "snapshot-id"),
        ("sourceSandboxId", image.source_vm, "snapshot-source"),
    ] {
        match row[field].as_str() {
            None => return Err(Unverified("snapshot-unreadable")),
            Some(value) if value != expected => return Err(Mismatch(check)),
            Some(_) => (),
        }
    }
    match &row["type"] {
        Value::Null => (),
        Value::String(kind) if kind == image.machine.as_str() => (),
        Value::String(_) => return Err(Mismatch("snapshot-type")),
        _ => return Err(Unverified("snapshot-unreadable")),
    }
    if row["status"] != "ready" {
        return Err(Unverified("snapshot-not-ready"));
    }
    Ok(())
}

/// Classify the guest marker check's exit status; anything unexpected is unverified.
pub(super) fn marker_status(code: Result<u8, String>) -> Result<(), Refusal> {
    match code {
        Ok(0) => Ok(()),
        Ok(95) => Err(Refusal::Mismatch("recipe-marker")),
        _ => Err(Refusal::Unverified("recipe-marker")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::memory::Class;
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    const LARGE: &Image = &QUALIFIED[0];

    fn config(large_max_memory_gib: u8) -> BoatConfig {
        BoatConfig {
            large_max_memory_gib,
            ..BoatConfig::default()
        }
    }

    fn limit(cap_gib: u8) -> Limit {
        Limit {
            class: Class::Default,
            cap_gib,
        }
    }

    #[test]
    fn selection_uses_only_pinned_images_and_names_the_missing_pin() {
        assert_eq!(select(limit(6), &config(0)), Err("image-unpinned"));
        assert_eq!(select(limit(6), &config(8)), Ok(LARGE));
        assert_eq!(select(limit(8), &config(8)), Ok(LARGE));
        assert_eq!(select(limit(9), &config(8)), Err("unqualified-ceiling"));
        assert_eq!(select(limit(9), &config(0)), Err("unqualified-ceiling"));
        static BOTH: [Image; 2] = [
            Image {
                name: "default-image",
                machine: Machine::Default,
                ..QUALIFIED[0]
            },
            Image {
                name: "large-image",
                ..QUALIFIED[0]
            },
        ];
        assert_eq!(
            select_from(6, &config(8), &BOTH).unwrap().name,
            "default-image"
        );
        assert_eq!(
            select_from(7, &config(8), &BOTH).unwrap().name,
            "large-image"
        );
        static DEFAULT_ONLY: [Image; 1] = [Image {
            machine: Machine::Default,
            ..QUALIFIED[0]
        }];
        assert_eq!(
            select_from(8, &config(12), &DEFAULT_ONLY),
            Err("image-unpinned")
        );
    }

    fn row() -> Value {
        json!({"name": LARGE.name, "snapshotId": LARGE.snapshot_id,
            "sourceSandboxId": LARGE.source_vm, "status": "ready", "type": "large"})
    }

    fn listed(rows: Vec<Value>) -> Value {
        json!({"named": rows, "namedAllowance": {"included": 10, "used": 1, "extra": 0}})
    }

    #[test]
    fn snapshot_rows_verify_only_the_exact_ready_pinned_image() {
        use Refusal::{Mismatch, Unverified};
        let other = json!({"name": "eon-warm", "snapshotId": "x", "sourceSandboxId": "bx_y",
            "status": "ready"});
        assert_eq!(verify(&listed(vec![other.clone(), row()]), LARGE), Ok(()));
        let mut untyped = row();
        untyped["type"] = Value::Null;
        assert_eq!(verify(&listed(vec![untyped]), LARGE), Ok(()));
        let edit = |pointer: &str, value: Value| {
            let mut changed = row();
            *changed.pointer_mut(pointer).unwrap() = value;
            verify(&listed(vec![changed]), LARGE)
        };
        for (pointer, value, refusal) in [
            (
                "/snapshotId",
                json!("9c882067-0000-4000-8000-000000000000"),
                Mismatch("snapshot-id"),
            ),
            (
                "/sourceSandboxId",
                json!("bx_other"),
                Mismatch("snapshot-source"),
            ),
            ("/type", json!("default"), Mismatch("snapshot-type")),
            (
                "/snapshotId",
                Value::Null,
                Unverified("snapshot-unreadable"),
            ),
            (
                "/sourceSandboxId",
                json!(7),
                Unverified("snapshot-unreadable"),
            ),
            ("/type", json!(["large"]), Unverified("snapshot-unreadable")),
            ("/status", json!("saving"), Unverified("snapshot-not-ready")),
            ("/status", Value::Null, Unverified("snapshot-not-ready")),
        ] {
            assert_eq!(
                edit(pointer, value.clone()),
                Err(refusal),
                "{pointer} {value}"
            );
        }
        assert_eq!(
            verify(&listed(vec![other]), LARGE),
            Err(Unverified("snapshot-absent"))
        );
        assert_eq!(
            verify(&listed(vec![row(), row()]), LARGE),
            Err(Mismatch("snapshot-duplicate"))
        );
        let mut wrong = row();
        wrong["snapshotId"] = json!("other");
        assert_eq!(
            verify(&listed(vec![row(), wrong]), LARGE),
            Err(Mismatch("snapshot-duplicate"))
        );
        assert_eq!(
            verify(&json!({"named": {}}), LARGE),
            Err(Unverified("snapshot-unreadable"))
        );
        assert_eq!(
            Refusal::Mismatch("recipe-marker").to_string(),
            "boat-image-mismatch check=recipe-marker"
        );
    }

    #[test]
    fn marker_statuses_split_mismatch_from_everything_unverifiable() {
        assert_eq!(marker_status(Ok(0)), Ok(()));
        assert_eq!(
            marker_status(Ok(95)),
            Err(Refusal::Mismatch("recipe-marker"))
        );
        for code in [Ok(1), Ok(94), Ok(255), Err("boat-client-timeout".into())] {
            assert_eq!(
                marker_status(code.clone()),
                Err(Refusal::Unverified("recipe-marker")),
                "{code:?}"
            );
        }
    }

    #[test]
    fn guest_marker_script_requires_the_exact_root_owned_read_only_file() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("recipe.sha256");
        let (uid, gid) = (
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw(),
        );
        // The test user cannot create a root-owned file; the owner rule is the same comparison
        // with the test user standing in for root.
        assert_eq!(MARKER.matches("0:0:444").count(), 1);
        let expecting = |owner: &str| {
            MARKER
                .replace("/opt/eon-boat/recipe.sha256", marker.to_str().unwrap())
                .replace("0:0:444", owner)
        };
        let script = expecting(&format!("{uid}:{gid}:444"));
        let run_script = |script: &str| {
            Command::new("bash")
                .args([
                    "--noprofile",
                    "--norc",
                    "-c",
                    script,
                    "eon-image",
                    LARGE.recipe_sha256,
                ])
                .env_clear()
                .env("PATH", std::env::var("PATH").unwrap())
                .status()
                .unwrap()
                .code()
        };
        let run = || run_script(&script);
        let write = |content: &str, mode: u32| {
            if marker.exists() {
                std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
                std::fs::remove_file(&marker).unwrap();
            }
            std::fs::write(&marker, content).unwrap();
            std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        assert_eq!(run(), Some(94), "absent");
        write(&format!("{}\n", LARGE.recipe_sha256), 0o444);
        assert_eq!(run(), Some(0), "verified");
        let other_user = expecting(&format!("{}:{gid}:444", uid.wrapping_add(1)));
        assert_eq!(run_script(&other_user), Some(95), "other owner");
        let other_group = expecting(&format!("{uid}:{}:444", gid.wrapping_add(1)));
        assert_eq!(run_script(&other_group), Some(95), "other group");
        write(LARGE.recipe_sha256, 0o444);
        assert_eq!(run(), Some(95), "missing newline");
        write(&format!("{}\n\n", LARGE.recipe_sha256), 0o444);
        assert_eq!(run(), Some(95), "extra line");
        write(&format!("{}\n", "0".repeat(64)), 0o444);
        assert_eq!(run(), Some(95), "other digest");
        write(&format!("{}\n", LARGE.recipe_sha256), 0o644);
        assert_eq!(run(), Some(95), "writable");
        write(&format!("{}\n", LARGE.recipe_sha256), 0o400);
        assert_eq!(run(), Some(95), "unreadable mode is also the wrong mode");
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&marker).unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, format!("{}\n", LARGE.recipe_sha256)).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
        symlink(&target, &marker).unwrap();
        assert_eq!(run(), Some(95), "symlink");
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert_eq!(run(), Some(95), "directory");
    }
}
