//! `[slots.boat]`, which rota keeps as an opaque table: Boat reads and checks it here.
use serde::Deserialize;
use std::collections::BTreeSet;

/// A command shape that `slots.boat.allow_shapes` can permit on Boat. The slot route
/// classifies each request's argv into at most one shape.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    NixGate,
    CargoBuild,
    CargoClippy,
    EonTest,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoatConfig {
    pub allow_shapes: Vec<Shape>,
    pub environment: String,
    pub reserve_hours: f64,
    pub timeout_seconds: u64,
    pub jobs: u16,
    /// Removed with warm reuse: parsed so older files load, ignored, and reported by
    /// [`BoatConfig::tombstone`]. Ignoring it is safe: nothing is reused either way.
    pub snapshot: Option<String>,
    pub large_max_memory_gib: u8,
}

impl BoatConfig {
    /// The warning for a removed key the file still sets; slot commands print it.
    pub fn tombstone(&self) -> Option<&'static str> {
        self.snapshot.is_some().then_some(
            "slots.boat.snapshot is removed and ignored; remote runs start fresh \
             from Eon's pinned qualified image",
        )
    }

    /// Reads `[slots.boat]` from a whole configuration file, with toml's line and column
    /// on a malformed key, and checks the policy.
    pub fn from_text(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            slots: Slots,
        }
        #[derive(Default, Deserialize)]
        struct Slots {
            #[serde(default)]
            boat: BoatConfig,
        }
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        file.slots.boat.validate()?;
        Ok(file.slots.boat)
    }

    /// Reads the table rota kept for the route, and checks the policy.
    pub fn from_table(table: &toml::Table) -> Result<Self, String> {
        let config =
            Self::deserialize(toml::Value::Table(table.clone())).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        let boat = self;
        if boat.allow_shapes.iter().collect::<BTreeSet<_>>().len() != boat.allow_shapes.len()
            || boat.environment.is_empty()
            || boat.environment == "base"
            || boat.environment.len() > 63
            || !boat
                .environment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            || !boat.reserve_hours.is_finite()
            || boat.reserve_hours < 0.0
            || !(120..=7200).contains(&boat.timeout_seconds)
            || !(1..=4).contains(&boat.jobs)
            || !(boat.large_max_memory_gib == 0 || (8..=12).contains(&boat.large_max_memory_gib))
        {
            return Err("invalid slots.boat policy: use unique build shapes, a non-base environment, finite reserve, timeout 120..7200 and jobs 1..4".into());
        }
        Ok(())
    }
}

impl Default for BoatConfig {
    fn default() -> Self {
        Self {
            allow_shapes: Vec::new(),
            environment: "eon-builds".into(),
            reserve_hours: 2.0,
            timeout_seconds: 3600,
            jobs: 4,
            snapshot: None,
            large_max_memory_gib: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boat_shapes_accept_each_known_name_once_and_refuse_unknown_or_repeated_names() {
        let with = |shapes: &str| format!("[slots.boat]\nallow_shapes = {shapes}\n");
        let config = BoatConfig::from_text(&with(
            "['nix-gate', 'cargo-build', 'cargo-clippy', 'eon-test']",
        ))
        .unwrap();
        assert_eq!(
            config.allow_shapes,
            [
                Shape::NixGate,
                Shape::CargoBuild,
                Shape::CargoClippy,
                Shape::EonTest
            ]
        );
        let error = BoatConfig::from_text(&with("['bogus']")).unwrap_err();
        assert!(error.contains("unknown variant `bogus`"), "{error}");
        let error = BoatConfig::from_text(&with("['cargo-clippy', 'cargo-clippy']")).unwrap_err();
        assert!(error.contains("invalid slots.boat policy"), "{error}");
    }

    #[test]
    fn boat_snapshot_is_a_warned_tombstone() {
        let text =
            include_str!("../tests/fixtures/boat-snapshot.toml").replace("<ROOT>", "/scratch");
        let config = BoatConfig::from_text(&text).unwrap();
        assert_eq!(config.allow_shapes, [Shape::EonTest]);
        assert_eq!(
            config.tombstone(),
            Some(
                "slots.boat.snapshot is removed and ignored; remote runs start fresh \
                 from Eon's pinned qualified image"
            )
        );
        assert_eq!(
            BoatConfig::from_text("[slots.boat]\nallow_shapes = []\n")
                .unwrap()
                .tombstone(),
            None
        );
    }

    #[test]
    fn the_text_and_the_table_read_the_same_policy() {
        let text = "[slots]\ncount = 2\n[slots.boat]\nallow_shapes = ['cargo-build']\njobs = 2\n";
        let table: toml::Table = toml::from_str(text).unwrap();
        let table = table["slots"]["boat"].as_table().unwrap();
        let from_table = BoatConfig::from_table(table).unwrap();
        let from_text = BoatConfig::from_text(text).unwrap();
        assert_eq!(from_table.allow_shapes, from_text.allow_shapes);
        assert_eq!((from_table.jobs, from_text.jobs), (2, 2));
        let mut bad = table.clone();
        bad.insert("jobs".into(), 9.into());
        let error = BoatConfig::from_table(&bad).unwrap_err();
        assert!(error.contains("invalid slots.boat policy"), "{error}");
        bad.insert("unknown".into(), 1.into());
        bad.remove("jobs");
        let error = BoatConfig::from_table(&bad).unwrap_err();
        assert!(error.contains("unknown field `unknown`"), "{error}");
    }
}
