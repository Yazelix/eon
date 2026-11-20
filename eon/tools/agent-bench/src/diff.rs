//! Field differences between two records, as leaf paths (`tools.rtk.version`,
//! `prompt.fleet_rules[1].sha256`). Arrays compare by index, and an empty array or object is a
//! leaf of its own, so a field that empties out still shows.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// One differing path; `None` means the side lacks the path.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let side = |value: &Option<Value>| match value {
            Some(value) => value.to_string(),
            None => "(absent)".into(),
        };
        write!(
            f,
            "{}: {} -> {}",
            self.path,
            side(&self.old),
            side(&self.new)
        )
    }
}

/// The differing leaf paths of `old` and `new`, sorted by path.
pub fn diff(old: &Value, new: &Value) -> Vec<Change> {
    let (old, new) = (flatten(old), flatten(new));
    let paths: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    paths
        .into_iter()
        .filter(|path| old.get(*path) != new.get(*path))
        .map(|path| Change {
            path: path.clone(),
            old: old.get(path).cloned(),
            new: new.get(path).cloned(),
        })
        .collect()
}

fn flatten(value: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    walk(value, String::new(), &mut out);
    out
}

fn walk(value: &Value, path: String, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(members) if !members.is_empty() => {
            for (name, item) in members {
                let next = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                walk(item, next, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                walk(item, format!("{path}[{index}]"), out);
            }
        }
        _ => {
            out.insert(path, value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_changed_paths() {
        let old = json!({"a": 1, "b": {"c": "x", "d": [1, 2]}, "e": [], "gone": true});
        let new = json!({"a": 1, "b": {"c": "y", "d": [1, 2, 3]}, "e": ["z"], "rubric": "r"});
        let lines: Vec<String> = diff(&old, &new).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "b.c: \"x\" -> \"y\"",
                "b.d[2]: (absent) -> 3",
                "e: [] -> (absent)",
                "e[0]: (absent) -> \"z\"",
                "gone: true -> (absent)",
                "rubric: (absent) -> \"r\"",
            ]
        );
        assert!(diff(&old, &old).is_empty());
    }
}
