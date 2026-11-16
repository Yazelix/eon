#[path = "@launchModule@"]
mod launch_environment;
#[path = "@nativeModule@"]
mod native_command;

use launch_environment::{LaunchFacts, LaunchInputs};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::Path,
    process::{Command, Stdio},
};

fn facts(wrapper: &Path) -> Result<LaunchFacts, String> {
    let executable = wrapper.with_file_name(".eon-wrapped");
    let locator = wrapper
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("share/eon/launch-environment");
    LaunchFacts::decode(
        &fs::read(locator).map_err(|error| error.to_string())?,
        wrapper,
        &executable,
    )
}

fn baseline(
    case: usize,
    bash: &OsStr,
    chain: &[OsString],
    native: &OsStr,
) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = [
        ("PROBE_CASE".into(), case.to_string().into()),
        ("PROBE_BASH".into(), bash.into()),
        ("PROBE_NATIVE".into(), native.into()),
        ("EON_ORBIT".into(), "user orbit".into()),
        ("EON_HX".into(), "user hx".into()),
        ("n".into(), "user n".into()),
        (
            "BASH_FUNC_eon_probe%%".into(),
            "() { printf exported; }".into(),
        ),
        (
            "USER_VALUE".into(),
            OsString::from_vec(b"\xff\n'*=value".to_vec()),
        ),
    ]
    .into();
    if case != 0 {
        env.insert(
            "PATH".into(),
            if case == 1 {
                "".into()
            } else {
                "/user bin::/custom:/custom:'*=x".into()
            },
        );
        let value = if case == 1 {
            OsString::new()
        } else {
            let mut paths = Vec::new();
            for wrapper in chain {
                let table = fs::read(
                    Path::new(wrapper)
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join("share/eon/launch-environment"),
                )
                .unwrap();
                let tokens: Vec<_> = table.split(|byte| *byte == 0).collect();
                for fields in tokens[3..tokens.len() - 1].chunks_exact(3) {
                    if fields[0] == b"prefix" && fields[1] == b"TERMINFO_DIRS" {
                        paths.push(fields[2].to_vec());
                    }
                }
            }
            let mut bytes = paths.join(&b':');
            bytes.extend_from_slice(b"::/user\xff\n'*=x:/user");
            OsString::from_vec(bytes)
        };
        env.insert("TERMINFO_DIRS".into(), value);
    }
    env
}

fn assert_baseline(launch: &LaunchInputs, mut expected: BTreeMap<OsString, OsString>) {
    let prepared = launch.command(launch.facts());
    let mut restored: BTreeMap<_, _> = prepared
        .get_envs()
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value.to_owned())))
        .collect();
    expected.remove(OsStr::new("BASH_FUNC_eon_probe%%"));
    restored.remove(OsStr::new("BASH_FUNC_eon_probe%%"));
    restored.remove(OsStr::new("PROBE_SERIALIZED"));
    assert_eq!(restored, expected);
}

fn reenter(args: &[OsString]) -> Result<(), String> {
    let wrapper = std::env::current_exe().unwrap().with_file_name("eon");
    let environment: BTreeMap<_, _> = std::env::vars_os().collect();
    let launch = LaunchInputs::capture(&facts(&wrapper)?, args.to_vec(), environment.clone())?;
    let expected = &environment[OsStr::new("PROBE_EXPECT_TERM")];
    assert_eq!(&environment[OsStr::new("TERMINFO_DIRS")], expected);
    let prepared = launch.command(launch.facts());
    assert!(
        prepared
            .get_envs()
            .any(|(key, value)| key == "TERMINFO_DIRS" && value == Some(expected.as_os_str()))
    );
    assert!(
        prepared
            .get_envs()
            .any(|(key, value)| key == "PROBE_EDIT" && value == Some(OsStr::new("kept")))
    );
    assert!(
        prepared
            .get_envs()
            .any(|(key, value)| key == "PATH" && value == Some(OsStr::new("/shell/edit::/path")))
    );
    assert!(prepared.get_envs().any(|(key, value)| key == "EON_ORBIT"
        && value == Some(environment[OsStr::new("EON_ORBIT")].as_os_str())));
    let count: u8 = args[2].to_str().unwrap().parse().unwrap();
    if count > 0 {
        assert!(
            launch
                .command(launch.facts())
                .args(["reenter", &(count - 1).to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    Ok(())
}

fn native(args: &[OsString]) -> Result<(), String> {
    let wrapper = std::env::var_os("EON_NEW_WINDOW_EXECUTABLE").unwrap();
    let launch = native_command::WindowCommand::new(wrapper.clone().into(), std::env::vars_os());
    let mut arguments = vec![OsString::from("check-native")];
    arguments.extend_from_slice(&args[2..]);
    assert!(
        launch
            .command()?
            .args(&arguments)
            .status()
            .unwrap()
            .success()
    );
    // A raw CLI loses capture; a wrapper-only fix captures expanded Venus values.
    for (index, program) in [
        Path::new(&wrapper).with_file_name(".eon-wrapped"),
        wrapper.into(),
    ]
    .into_iter()
    .enumerate()
    {
        let output = native_command::external_command(program)
            .env("EON_CONFIG_HOME", "/native/config")
            .env("EON_WINDOW_RUNTIME_DIR", "/native/window")
            .args(&arguments)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "native fallback unexpectedly preserved the baseline"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(if index == 0 {
            "missing or contradictory installed launch capture"
        } else {
            "EON_VENUS_FONT_DIRS"
        }));
    }
    assert!(
        native_command::external_command(std::env::current_exe().unwrap())
            .arg("check-private")
            .status()
            .unwrap()
            .success()
    );
    Ok(())
}

fn check_native(args: &[OsString]) -> Result<(), String> {
    let wrapper = std::env::current_exe().unwrap().with_file_name("eon");
    let environment: BTreeMap<_, _> = std::env::vars_os().collect();
    let launch = LaunchInputs::capture(&facts(&wrapper)?, args.to_vec(), environment.clone())?;
    let case = environment[OsStr::new("PROBE_CASE")]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut expected = baseline(
        case,
        &environment[OsStr::new("PROBE_BASH")],
        &args[2..],
        &environment[OsStr::new("PROBE_NATIVE")],
    );
    expected.insert("EON_CONFIG_HOME".into(), "/native/config".into());
    assert_baseline(&launch, expected);
    assert_eq!(
        environment[OsStr::new("EON_WINDOW_RUNTIME_DIR")],
        "/native/window"
    );
    assert!(
        !environment
            .keys()
            .any(|key| key.as_bytes().starts_with(b"EON_NEW_WINDOW_"))
    );
    Ok(())
}

fn hop(arguments: Vec<OsString>) -> Result<(), String> {
    let executable = std::env::current_exe().unwrap();
    let wrapper = executable.with_file_name("eon");
    let chain = &arguments[2..];
    let stage = chain
        .iter()
        .position(|path| Path::new(path) == wrapper)
        .unwrap();
    let before: BTreeMap<_, _> = std::env::vars_os().collect();
    let launch = LaunchInputs::capture(&facts(&wrapper)?, arguments.clone(), before.clone())?;
    let case = before[OsStr::new("PROBE_CASE")]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_baseline(
        &launch,
        baseline(
            case,
            &before[OsStr::new("PROBE_BASH")],
            chain,
            &before[OsStr::new("PROBE_NATIVE")],
        ),
    );
    let exported = Command::new(&before[OsStr::new("PROBE_BASH")])
        .args(["--noprofile", "--norc", "-c", "eon_probe"])
        .output()
        .unwrap();
    assert!(exported.status.success());
    assert_eq!(exported.stdout, b"exported");
    let serialized = &before[OsStr::new("BASH_FUNC_eon_probe%%")];
    if let Some(previous) = before.get(OsStr::new("PROBE_SERIALIZED")) {
        assert_eq!(
            serialized, previous,
            "exported function serialization accumulated"
        );
    }
    if stage > 0 {
        for name in [
            "EON_ROUTED",
            "EON_ATTACH_DELEGATED",
            "EON_TERMINAL",
            "EON_TAB",
            "EON_WINDOW_RUNTIME_DIR",
        ] {
            assert!(!before.contains_key(OsStr::new(name)));
        }
        assert_ne!(before.get(OsStr::new("SHLVL")), Some(&OsString::from("99")));
    }
    let mut old_shell = if stage == 0 {
        for alias in ["eon-bash", "hx"] {
            assert!(
                Command::new(wrapper.with_file_name(alias))
                    .env("PROBE_EXPECT_TERM", &before[OsStr::new("TERMINFO_DIRS")])
                    .env("PROBE_EDIT", "kept")
                    .env("PATH", "/shell/edit::/path")
                    .args(["reenter", "2"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let mut native = Command::new(&before[OsStr::new("PROBE_NATIVE")]);
        native
            .env("EON_NEW_WINDOW_ENV_deadbeef", "stale")
            .env("EON_NEW_WINDOW_ENVIRONMENT", "broken");
        launch.native_window_environment(
            &mut native,
            Path::new("/native/config"),
            Path::new("/native/window"),
        );
        assert!(native.arg("native").args(chain).status().unwrap().success());
        let mut command = Command::new(&before[OsStr::new("PROBE_BASH")]);
        launch_environment::remove_launch_markers(&mut command);
        Some(command.args(["--noprofile", "--norc", "-c", "test -z \"${!EON_LAUNCH_@}\" || exit 1; printf '%s\\n' \"${TERMINFO_DIRS-unset}\"; read -r; printf '%s\\n' \"${TERMINFO_DIRS-unset}\""])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap())
    } else {
        None
    };
    if stage + 1 < chain.len() {
        let target = facts(Path::new(&chain[stage + 1]))?;
        let status = launch
            .command(&target)
            .env("PROBE_SERIALIZED", serialized)
            .args(&launch.arguments()[1..])
            .status()
            .unwrap();
        assert!(status.success());
    }
    if let Some(mut shell) = old_shell.take() {
        shell.stdin.take().unwrap().write_all(b"done\n").unwrap();
        let output = shell.wait_with_output().unwrap();
        assert!(output.status.success());
        let mut expected = before
            .get(OsStr::new("TERMINFO_DIRS"))
            .cloned()
            .unwrap_or_else(|| "unset".into())
            .into_vec();
        expected.push(b'\n');
        expected.extend_from_within(..);
        assert_eq!(output.stdout, expected);
    }
    assert_eq!(std::env::vars_os().collect::<BTreeMap<_, _>>(), before);
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args[1] == "driver" {
        for chain in args[3..9].chunks_exact(3) {
            for case in 0..3 {
                let mut command = Command::new(&chain[0]);
                command
                    .env_clear()
                    .envs(baseline(case, &args[2], chain, &args[10]))
                    .env("EON_LAUNCH_SAVED_STALE", "+must disappear")
                    .env("SHLVL", "99")
                    .env("EON_ROUTED", "old")
                    .env("EON_ATTACH_DELEGATED", "old")
                    .env("EON_TERMINAL", "old")
                    .env("EON_TAB", "old")
                    .env("EON_WINDOW_RUNTIME_DIR", "/old/namespace")
                    .arg("hop")
                    .args(chain);
                assert!(command.status().unwrap().success());
            }
        }
        let old = &args[9];
        let output = Command::new(old)
            .env_clear()
            .envs(baseline(0, &args[2], std::slice::from_ref(old), &args[10]))
            .arg("hop")
            .arg(old)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("missing or contradictory installed launch capture")
        );
        println!(
            "six A→B→C chains passed; native baseline survives and both fallbacks refuse; re-entry is idempotent; running shells unchanged; old renderer refused"
        );
    } else {
        let result = match args[1].to_str().unwrap() {
            "reenter" => reenter(&args),
            "native" => native(&args),
            "check-native" => check_native(&args),
            "check-private" => {
                assert!(
                    !std::env::vars_os()
                        .any(|(key, _)| key.as_bytes().starts_with(b"EON_NEW_WINDOW_")
                            || key.as_bytes().starts_with(b"EON_LAUNCH_"))
                );
                Ok(())
            }
            _ => hop(args),
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
