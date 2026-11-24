//! Exact composition capture; filesystem and installed-build association belong to Eon.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt::Write,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
};

pub const CAPTURE_PREFIX: &str = "EON_LAUNCH_";
pub const FACTS_VARIABLE: &str = "EON_LAUNCH_FACTS";
pub const FACTS_LIMIT: usize = 65_536;
pub(crate) const ROUTE_MARKER: &str = "EON_ROUTED";
pub(crate) const DELEGATION_MARKER: &str = "EON_ATTACH_DELEGATED";
pub(crate) const WAYLAND_SOCKET: &str = "WAYLAND_SOCKET";
pub(crate) const TERMINAL_VARIABLE: &str = "EON_TERMINAL";
pub(crate) const TAB_VARIABLE: &str = "EON_TAB";

#[derive(Clone, Debug)]
struct Operation {
    kind: String,
    name: OsString,
    value: OsString,
}

/// Composition facts read from the installed target, not from a caller's environment.
#[derive(Clone, Debug)]
pub struct LaunchFacts {
    wrapper: PathBuf,
    locator: PathBuf,
    operations: Vec<Operation>,
}

impl LaunchFacts {
    /// Eon validates these two installed paths before decoding their bounded facts file.
    pub fn decode(bytes: &[u8], wrapper: &Path, executable: &Path) -> Result<Self, String> {
        let invalid = || "invalid installed launch facts".to_owned();
        if bytes.len() > FACTS_LIMIT || !bytes.ends_with(&[0]) {
            return Err(invalid());
        }
        let tokens: Vec<_> = bytes[..bytes.len() - 1].split(|byte| *byte == 0).collect();
        if tokens.len() < 3
            || tokens[0] != b"EON-LAUNCH-1"
            || !wrapper.is_absolute()
            || !executable.is_absolute()
            || tokens[1] != wrapper.as_os_str().as_bytes()
            || tokens[2] != executable.as_os_str().as_bytes()
            || !(tokens.len() - 3).is_multiple_of(3)
        {
            return Err(invalid());
        }
        let mut operations = Vec::<Operation>::new();
        for fields in tokens[3..].as_chunks::<3>().0 {
            let name = fields[1];
            if name.is_empty()
                || !(name[0].is_ascii_alphabetic() || name[0] == b'_')
                || !name
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                || name.starts_with(CAPTURE_PREFIX.as_bytes())
                || matches!(name, b"_" | b"PWD" | b"SHLVL")
                || operations.iter().any(|op| op.name.as_bytes() == name)
            {
                return Err(invalid());
            }
            let kind = match fields[0] {
                b"set" => "set",
                b"unset" if fields[2].is_empty() => "unset",
                b"prefix" if !fields[2].is_empty() => "prefix",
                _ => return Err(invalid()),
            };
            operations.push(Operation {
                kind: kind.into(),
                name: OsString::from_vec(name.to_vec()),
                value: OsString::from_vec(fields[2].to_vec()),
            });
        }
        let root = wrapper
            .parent()
            .and_then(Path::parent)
            .ok_or_else(invalid)?;
        Ok(Self {
            wrapper: wrapper.into(),
            locator: root.join("share/eon/launch-environment"),
            operations,
        })
    }

    pub(crate) fn wrapper(&self) -> &Path {
        &self.wrapper
    }

    /// An absolute executable locator declared by the installed composition.
    pub fn program(&self, name: &str) -> Option<&Path> {
        self.operations
            .iter()
            .find(|operation| operation.name == name && operation.kind == "set")
            .map(|operation| Path::new(&operation.value))
            .filter(|path| path.is_absolute())
    }
}

/// Original supported user inputs. Interpreter bookkeeping is recreated per wrapper.
#[derive(Debug)]
pub struct LaunchInputs {
    facts: LaunchFacts,
    arguments: Vec<OsString>,
    environment: BTreeMap<OsString, OsString>,
}

impl LaunchInputs {
    pub fn capture(
        facts: &LaunchFacts,
        arguments: Vec<OsString>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, String> {
        let mut environment: BTreeMap<_, _> = environment.into_iter().collect();
        if arguments.is_empty()
            || environment.get(OsStr::new(FACTS_VARIABLE))
                != Some(&facts.locator.clone().into_os_string())
        {
            return Err("missing or contradictory installed launch capture".into());
        }
        for operation in &facts.operations {
            let mut backup = OsString::from("EON_LAUNCH_SAVED_");
            backup.push(&operation.name);
            let original = match environment.get(&backup).map(|value| value.as_bytes()) {
                Some(b"-") => None,
                Some([b'+', value @ ..]) => Some(OsString::from_vec(value.to_vec())),
                _ => {
                    return Err(format!(
                        "missing or invalid launch backup for {:?}",
                        operation.name
                    ));
                }
            };
            let expected = match operation.kind.as_str() {
                "set" => Some(operation.value.clone()),
                "unset" => None,
                "prefix" => Some(prefix_value(&operation.value, original.as_deref())),
                _ => unreachable!(),
            };
            if environment.get(&operation.name) != expected.as_ref() {
                return Err(format!(
                    "contradictory launch value for {:?}",
                    operation.name
                ));
            }
            match original {
                Some(value) => {
                    environment.insert(operation.name.clone(), value);
                }
                None => {
                    environment.remove(&operation.name);
                }
            }
        }
        environment.retain(|name, _| {
            !name.as_bytes().starts_with(CAPTURE_PREFIX.as_bytes())
                && !name.as_bytes().starts_with(b"EON_NEW_WINDOW_")
                && !matches!(name.as_bytes(), b"_" | b"PWD" | b"SHLVL")
        });
        if cfg!(target_os = "linux") {
            environment.remove(OsStr::new(WAYLAND_SOCKET));
        }
        Ok(Self {
            facts: facts.clone(),
            arguments,
            environment,
        })
    }

    pub fn facts(&self) -> &LaunchFacts {
        &self.facts
    }

    pub fn caller_terminfo_dirs(&self) -> Option<&OsStr> {
        self.environment
            .get(OsStr::new("TERMINFO_DIRS"))
            .map(OsString::as_os_str)
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Native CLI children use this saved baseline, not Venus's expanded environment.
    pub fn native_window_environment(
        &self,
        command: &mut Command,
        config: &Path,
        namespace: &Path,
    ) {
        clear_native_window_environment(command);
        remove_launch_capture(command);
        let mut baseline = self.environment.clone();
        baseline.retain(|name, _| {
            !matches!(
                name.as_bytes(),
                b"EON_ROUTED"
                    | b"EON_ATTACH_DELEGATED"
                    | b"EON_TERMINAL"
                    | b"EON_TAB"
                    | b"EON_WINDOW_RUNTIME_DIR"
            )
        });
        baseline.insert("EON_CONFIG_HOME".into(), config.into());
        baseline.insert("EON_WINDOW_RUNTIME_DIR".into(), namespace.into());
        command
            .env("EON_NEW_WINDOW_EXECUTABLE", &self.facts.wrapper)
            .env("EON_NEW_WINDOW_ENVIRONMENT", "1");
        for (name, value) in baseline {
            let mut key = "EON_NEW_WINDOW_ENV_".to_owned();
            for byte in name.as_bytes() {
                write!(key, "{byte:02x}").expect("writing to a String");
            }
            command.env(key, value);
        }
    }

    /// Prepare only. The adoption owner supplies its selected namespace and entry arguments.
    pub fn command(&self, target: &LaunchFacts) -> Command {
        let mut command = Command::new(&target.wrapper);
        command.env_clear().envs(&self.environment);
        remove_launch_markers(&mut command);
        command.env_remove("EON_WINDOW_RUNTIME_DIR");
        command
    }
}

fn prefix_value(prefix: &OsStr, original: Option<&OsStr>) -> OsString {
    if let Some(original) = original {
        let needle = [&b":"[..], prefix.as_bytes(), &b":"[..]].concat();
        let entries = [&b":"[..], original.as_bytes(), &b":"[..]].concat();
        if entries
            .windows(needle.len())
            .any(|entry| entry == needle.as_slice())
        {
            return original.into();
        }
    }
    let mut value = prefix.to_os_string();
    if let Some(original) = original {
        value.push(":");
        value.push(original);
    }
    value
}

pub(crate) fn clear_native_window_environment(command: &mut Command) {
    let names: Vec<_> = std::env::vars_os()
        .map(|(name, _)| name)
        .chain(command.get_envs().map(|(name, _)| name.to_owned()))
        .filter(|name| name.as_bytes().starts_with(b"EON_NEW_WINDOW_"))
        .collect();
    for name in names {
        command.env_remove(name);
    }
}

/// Capture is private transport, including on a wrapper's managed-tool dispatch.
pub fn remove_launch_capture(command: &mut Command) -> &mut Command {
    let names: Vec<_> = std::env::vars_os()
        .map(|(name, _)| name)
        .chain(command.get_envs().map(|(name, _)| name.to_owned()))
        .filter(|name| name.as_bytes().starts_with(CAPTURE_PREFIX.as_bytes()))
        .collect();
    for name in names {
        command.env_remove(name);
    }
    command
}

pub(crate) fn remove_launch_markers(command: &mut Command) -> &mut Command {
    remove_launch_capture(command);
    for name in [
        ROUTE_MARKER,
        DELEGATION_MARKER,
        TERMINAL_VARIABLE,
        TAB_VARIABLE,
    ] {
        command.env_remove(name);
    }
    command
}

/// A connected display descriptor belongs to the initial Venus alone.
pub(crate) fn remove_wayland_socket(command: &mut Command) -> &mut Command {
    if cfg!(target_os = "linux") {
        command.env_remove(WAYLAND_SOCKET);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> LaunchFacts {
        LaunchFacts::decode(b"EON-LAUNCH-1\0/a/bin/eon\0/a/bin/.eon-wrapped\0set\0EON_TOOL\0/a/tool\0prefix\0TERMINFO_DIRS\0/a/info\0unset\0EON_HX\0\0", Path::new("/a/bin/eon"), Path::new("/a/bin/.eon-wrapped")).unwrap()
    }

    #[test]
    fn capture_restores_user_bytes_and_prepares_without_transient_authority() {
        let existing = OsString::from_vec(b"/a/info::/user\xff:\n'*=x".to_vec());
        for (original, expanded) in [
            (None, OsString::from("/a/info")),
            (Some(OsString::new()), OsString::from("/a/info:")),
            (
                Some(OsString::from("/user/info")),
                OsString::from("/a/info:/user/info"),
            ),
            (Some(existing.clone()), existing),
        ] {
            let mut environment: BTreeMap<OsString, OsString> = [
                (
                    FACTS_VARIABLE.into(),
                    "/a/share/eon/launch-environment".into(),
                ),
                ("EON_TOOL".into(), "/a/tool".into()),
                ("EON_LAUNCH_SAVED_EON_TOOL".into(), "+user tool".into()),
                ("EON_LAUNCH_SAVED_EON_HX".into(), "+user hx".into()),
                ("PATH".into(), "/user::/user".into()),
                (ROUTE_MARKER.into(), "old".into()),
                (DELEGATION_MARKER.into(), "old".into()),
                (TERMINAL_VARIABLE.into(), "old".into()),
                (TAB_VARIABLE.into(), "old".into()),
                ("SHLVL".into(), "99".into()),
                ("PWD".into(), "/old".into()),
                ("_".into(), "/old/bin/eon".into()),
            ]
            .into();
            let mut backup = OsString::from("-");
            if let Some(value) = &original {
                backup = OsString::from("+");
                backup.push(value);
            }
            environment.insert("TERMINFO_DIRS".into(), expanded);
            environment.insert("EON_LAUNCH_SAVED_TERMINFO_DIRS".into(), backup);
            let arguments = vec!["eon".into(), OsString::from_vec(vec![255])];
            let saved = LaunchInputs::capture(&facts(), arguments, environment.clone()).unwrap();
            assert_eq!(
                saved.environment.get(OsStr::new("TERMINFO_DIRS")),
                original.as_ref()
            );
            assert_eq!(saved.environment[OsStr::new("EON_TOOL")], "user tool");
            assert_eq!(saved.environment[OsStr::new("EON_HX")], "user hx");
            assert_eq!(saved.environment[OsStr::new("PATH")], "/user::/user");
            let mut command = saved.command(saved.facts());
            command.args(&saved.arguments()[1..]);
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [OsStr::from_bytes(&[255])]
            );
            for (name, value) in command.get_envs() {
                if matches!(
                    name.as_bytes(),
                    b"EON_ROUTED"
                        | b"EON_ATTACH_DELEGATED"
                        | b"EON_TERMINAL"
                        | b"EON_TAB"
                        | b"EON_WINDOW_RUNTIME_DIR"
                ) {
                    assert!(value.is_none());
                }
                if name.as_bytes().starts_with(CAPTURE_PREFIX.as_bytes()) {
                    assert!(value.is_none());
                }
            }
            environment.insert("EON_TOOL".into(), "/contradiction".into());
            assert!(LaunchInputs::capture(&facts(), vec!["eon".into()], environment).is_err());
        }
    }

    #[test]
    fn launch_facts_refuse_missing_malformed_and_conflicting_inputs() {
        let valid = b"EON-LAUNCH-1\0/a/bin/eon\0/a/bin/.eon-wrapped\0";
        let decode = |bytes: &[u8]| {
            LaunchFacts::decode(
                bytes,
                Path::new("/a/bin/eon"),
                Path::new("/a/bin/.eon-wrapped"),
            )
        };
        for bytes in [
            b"".as_slice(),
            b"EON-LAUNCH-2\0/a/bin/eon\0/a/bin/.eon-wrapped\0",
            &valid[..valid.len() - 1],
        ] {
            assert!(decode(bytes).is_err());
        }
        for suffix in [
            b"set\0A\0x\0set\0A\0y\0".as_slice(),
            b"unset\0A\0x\0",
            b"set\0EON_LAUNCH_X\0x\0",
            b"set\09bad\0x\0",
            b"unknown\0A\0x\0",
            b"prefix\0A\0\0",
            b"set\0A\0",
        ] {
            let mut bytes = valid.to_vec();
            bytes.extend_from_slice(suffix);
            assert!(decode(&bytes).is_err());
        }
        assert!(decode(&vec![0; FACTS_LIMIT + 1]).is_err());
        assert!(LaunchInputs::capture(&facts(), vec!["eon".into()], []).is_err());
    }

    #[test]
    fn prefix_reentry_keeps_existing_user_order_and_duplicates() {
        let facts = LaunchFacts::decode(
            b"EON-LAUNCH-1\0/a/bin/eon\0/a/bin/.eon-wrapped\0prefix\0TERMINFO_DIRS\0/a/info\0",
            Path::new("/a/bin/eon"),
            Path::new("/a/bin/.eon-wrapped"),
        )
        .unwrap();
        for original in ["/a/info", "/user:/a/info", "/a/info::/a/info:/user"] {
            let capture = LaunchInputs::capture(
                &facts,
                vec!["eon".into()],
                [
                    (
                        FACTS_VARIABLE.into(),
                        "/a/share/eon/launch-environment".into(),
                    ),
                    ("TERMINFO_DIRS".into(), original.into()),
                    (
                        "EON_LAUNCH_SAVED_TERMINFO_DIRS".into(),
                        format!("+{original}").into(),
                    ),
                ],
            )
            .unwrap();
            assert_eq!(capture.environment[OsStr::new("TERMINFO_DIRS")], original);
        }
    }
}
