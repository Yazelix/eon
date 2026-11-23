//! Eon's private native CLI launch inputs, distinct from Venus's own wrapper environment.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    process::Command,
};

const VERSION: &str = "EON_NEW_WINDOW_ENVIRONMENT";
const PREFIX: &str = "EON_NEW_WINDOW_ENV_";

#[derive(Clone, Debug)]
pub struct WindowCommand {
    executable: PathBuf,
    environment: Result<Option<BTreeMap<OsString, OsString>>, String>,
}

impl WindowCommand {
    pub fn new(
        executable: PathBuf,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        Self {
            executable,
            environment: decode(environment),
        }
    }

    pub fn command(&self) -> Result<Command, String> {
        let environment = self.environment.as_ref().map_err(Clone::clone)?;
        let mut command = external_command(&self.executable);
        if let Some(environment) = environment {
            command.env_clear().envs(environment);
        }
        Ok(command)
    }
}

/// Browser/desktop children must not inherit Eon's private native launch snapshot.
pub fn external_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        if name.as_bytes().starts_with(b"EON_NEW_WINDOW_") {
            command.env_remove(name);
        }
    }
    command
}

fn decode(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Option<BTreeMap<OsString, OsString>>, String> {
    let invalid = || "Cannot run Eon window command: invalid private launch environment".to_owned();
    let mut version = None;
    let mut baseline = BTreeMap::new();
    for (name, value) in environment {
        if name == VERSION {
            if version.replace(value).is_some() {
                return Err(invalid());
            }
        } else if let Some(hex) = name.as_bytes().strip_prefix(PREFIX.as_bytes()) {
            if hex.is_empty() || !hex.len().is_multiple_of(2) {
                return Err(invalid());
            }
            let mut bytes = Vec::new();
            for pair in hex.as_chunks::<2>().0 {
                let nibble = |byte| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    _ => None,
                };
                bytes.push(
                    nibble(pair[0]).ok_or_else(invalid)? * 16
                        + nibble(pair[1]).ok_or_else(invalid)?,
                );
            }
            if bytes.contains(&0) || bytes.contains(&b'=') || value.as_bytes().contains(&0) {
                return Err(invalid());
            }
            if baseline.insert(OsString::from_vec(bytes), value).is_some() {
                return Err(invalid());
            }
        }
    }
    match version {
        None if baseline.is_empty() => Ok(None),
        Some(version) if version == "1" => {
            baseline.retain(|name, _| {
                !name.as_bytes().starts_with(b"EON_NEW_WINDOW_")
                    && !name.as_bytes().starts_with(b"EON_LAUNCH_")
                    && !matches!(
                        name.as_bytes(),
                        b"EON_TERMINAL" | b"EON_TAB" | b"EON_ROUTED" | b"EON_ATTACH_DELEGATED"
                    )
            });
            Ok(Some(baseline))
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_window_uses_baseline_and_refuses_partial_or_unknown_handoff() {
        let command = WindowCommand::new(
            "/bin/sh".into(),
            [
                (VERSION.into(), "1".into()),
                ("EON_NEW_WINDOW_ENV_55534552".into(), "original user".into()),
                ("EON_NEW_WINDOW_ENV_454d505459".into(), "".into()),
                (
                    "EON_NEW_WINDOW_ENV_ff".into(),
                    OsString::from_vec(vec![255]),
                ),
                ("EON_ORBIT".into(), "expanded Venus environment".into()),
            ],
        );
        let mut child = command.command().unwrap();
        assert!(child.get_envs().any(|(name,value)| name.as_bytes() == [255] && value.unwrap().as_bytes() == [255]));
        let output = child.args(["-c", "test \"$USER\" = 'original user' && test \"${EMPTY+x}\" = x && test -z \"${EON_ORBIT+x}\" && test -z \"${EON_NEW_WINDOW_ENVIRONMENT+x}\" && printf clean"]).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"clean");
        for fields in [
            vec![(VERSION, "2")],
            vec![("EON_NEW_WINDOW_ENV_55", "user")],
            vec![(VERSION, "1"), ("EON_NEW_WINDOW_ENV_", "x")],
            vec![(VERSION, "1"), ("EON_NEW_WINDOW_ENV_FF", "x")],
            vec![(VERSION, "1"), ("EON_NEW_WINDOW_ENV_3d", "x")],
            vec![(VERSION, "1"), ("EON_NEW_WINDOW_ENV_00", "x")],
        ] {
            assert!(
                WindowCommand::new(
                    "/must-not-run".into(),
                    fields.into_iter().map(|(k, v)| (k.into(), v.into()))
                )
                .command()
                .is_err()
            );
        }
    }
}
