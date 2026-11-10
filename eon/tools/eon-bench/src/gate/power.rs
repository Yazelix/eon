use super::{Result, error, field, object};
use crate::json::Value;
use std::{collections::BTreeMap, fs, path::Path};

fn read(root: &Path) -> Result<Value> {
    let mut supplies = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(error)? {
        let path = entry.map_err(error)?.path();
        if fs::read_to_string(path.join("type")).map_err(error)?.trim() != "Mains" {
            continue;
        }
        let online = match fs::read_to_string(path.join("online"))
            .map_err(error)?
            .trim()
        {
            "0" => false,
            "1" => true,
            _ => return Err(format!("invalid AC online state: {}", path.display())),
        };
        supplies.insert(
            path.file_name().unwrap().to_string_lossy().into_owned(),
            online.into(),
        );
    }
    let value = Value::Object(supplies.into_iter().collect());
    state(&value)?;
    Ok(value)
}

fn state(value: &Value) -> Result<BTreeMap<&str, bool>> {
    let mut supplies = BTreeMap::new();
    for (name, online) in value.as_object().ok_or("AC state is not an object")? {
        if name.is_empty()
            || supplies
                .insert(name.as_str(), online.as_bool().ok_or("invalid AC state")?)
                .is_some()
        {
            return Err("invalid/duplicate AC supply".into());
        }
    }
    if supplies.is_empty() {
        return Err("no observable Mains power supply; timing is invalid".into());
    }
    Ok(supplies)
}

fn validate(start: &Value, end: &Value, baseline: Option<&Value>) -> Result {
    let start = state(start)?;
    if start != state(end)? {
        return Err("AC state changed during the run; timing is invalid".into());
    }
    if let Some(baseline) = baseline
        && start != state(baseline)?
    {
        return Err("AC state differs from baseline; timing is invalid".into());
    }
    Ok(())
}

pub(super) fn recorded(snapshot: &Value) -> Result<&Value> {
    let audit = field(snapshot, "ac_power")?;
    if field(audit, "error")? != &Value::Null {
        return Err("baseline has an invalid/incomplete power audit".into());
    }
    let start = field(audit, "start")?;
    validate(start, field(audit, "end")?, None)?;
    Ok(start)
}

/// Save endpoints even when a suite fails; an interrupted run retains a null end.
pub(super) fn measure<T>(
    root: &Path,
    audit_path: &Path,
    baseline: Option<&Value>,
    action: impl FnOnce() -> Result<T>,
) -> Result<(T, Value)> {
    let start = read(root);
    let audit = |end: Option<&Value>, failure: Option<&str>| {
        object([
            ("start", start.as_ref().cloned().unwrap_or(Value::Null)),
            ("end", end.cloned().unwrap_or(Value::Null)),
            ("error", failure.into()),
        ])
    };
    fs::write(audit_path, audit(None, Some("incomplete run")).to_pretty()).map_err(error)?;
    let outcome = (|| {
        let start = start.as_ref().map_err(Clone::clone)?;
        validate(start, start, baseline)?;
        action()
    })();
    let end = read(root);
    let check = (|| {
        validate(
            start.as_ref().map_err(Clone::clone)?,
            end.as_ref().map_err(Clone::clone)?,
            baseline,
        )
    })();
    let audit = audit(
        end.as_ref().ok(),
        check
            .as_ref()
            .err()
            .or(outcome.as_ref().err())
            .map(String::as_str),
    );
    fs::write(audit_path, audit.to_pretty()).map_err(error)?;
    check?;
    Ok((outcome?, audit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_endpoints_are_recorded_and_changes_or_mismatches_refuse_timing() {
        let root = std::env::temp_dir().join(format!("gate-power-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let ac = root.join("ACAD");
        let usb = root.join("usb");
        fs::create_dir(&ac).unwrap();
        fs::create_dir(&usb).unwrap();
        fs::write(ac.join("type"), "Mains\n").unwrap();
        fs::write(ac.join("online"), "1\n").unwrap();
        fs::write(usb.join("type"), "USB\n").unwrap();
        let audit_path = root.with_extension("json");
        let baseline = object([("ACAD", true.into())]);
        let run = |action: &mut dyn FnMut() -> Result| {
            measure(&root, &audit_path, Some(&baseline), action)
        };
        let (_, audit) = run(&mut || Ok(())).unwrap();
        assert_eq!(recorded(&object([("ac_power", audit)])).unwrap(), &baseline);
        assert!(
            run(&mut || {
                fs::write(ac.join("online"), "0\n").unwrap();
                Ok(())
            })
            .unwrap_err()
            .contains("changed")
        );
        let audit = crate::gate::read(&audit_path).unwrap();
        assert_eq!(audit.get("start"), Some(&baseline));
        assert_eq!(audit.get("end"), Some(&object([("ACAD", false.into())])));
        assert!(
            run(&mut || panic!("mismatched power must not launch suites"))
                .unwrap_err()
                .contains("differs")
        );
        fs::write(ac.join("online"), "1\n").unwrap();
        assert_eq!(
            run(&mut || Err("suite failed".into())).unwrap_err(),
            "suite failed"
        );
        assert_eq!(
            crate::gate::read(&audit_path).unwrap().get("end"),
            Some(&baseline)
        );
        fs::write(ac.join("online"), "unknown\n").unwrap();
        assert!(read(&root).is_err());
        fs::remove_dir_all(ac).unwrap();
        assert!(read(&root).is_err());
        for bad in [
            "null",
            "{}",
            "{\"ACAD\":1}",
            "{\"ACAD\":true,\"ACAD\":false}",
        ] {
            assert!(state(&Value::parse(bad).unwrap()).is_err());
        }
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(audit_path).unwrap();
    }
}
