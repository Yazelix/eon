//! The wrapper: `agent-jail` bound over every shell path and every tool
//! name on the outer layer's `PATH`. It moves the command into the inner
//! layer, a nested bwrap with its own network and PID namespaces, private
//! `/tmp`, `/run` and home, no proxy variables and no credential, where the
//! real tools are bound over the wrapper paths, so nothing re-wraps. Its
//! `PATH` adds the dev shell's store dirs after `/bench/bin`, so the dev
//! tools run only where commands already run, without wrappers.

use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;

use crate::spec::{PROXY_VARS, Spec};

/// Exit status when the inner layer could not be set up.
pub const SETUP_FAILED: i32 = 2;

/// Builds the inner bwrap's arguments for the wrapper at `exe`.
pub fn inner_args(
    spec: &Spec,
    exe: &str,
    argv0: &OsString,
    cwd: &Path,
    status_fd: i32,
) -> Result<Vec<OsString>, String> {
    if !spec.tools.contains_key(exe) {
        return Err(format!("{exe} is not a jail wrapper"));
    }
    let mut args: Vec<OsString> = [
        "--unshare-user",
        "--unshare-net",
        "--unshare-pid",
        "--unshare-ipc",
        "--die-with-parent",
        "--bind",
        "/",
        "/",
        "--proc",
        "/proc",
        // `--bind / /` remounts /dev nodev; shells need a working /dev/null.
        "--dev",
        "/dev",
        // The workspace as commands see it, links and all (see `seal`).
        "--bind",
        crate::INNER_WORKSPACE,
        crate::WORKSPACE,
    ]
    .map(OsString::from)
    .into();
    for mask in &spec.inner_masks {
        args.extend(["--tmpfs".into(), mask.into()]);
    }
    args.extend([
        "--ro-bind".into(),
        "/dev/null".into(),
        spec.auth_path.clone().into(),
    ]);
    for name in PROXY_VARS {
        args.extend(["--unsetenv".into(), (*name).into()]);
    }
    args.extend(["--setenv".into(), "PATH".into(), inner_path(spec).into()]);
    for (wrapper, real) in &spec.tools {
        args.extend(["--ro-bind".into(), real.into(), wrapper.into()]);
    }
    args.extend([
        "--json-status-fd".into(),
        status_fd.to_string().into(),
        "--chdir".into(),
        cwd.as_os_str().to_owned(),
        "--argv0".into(),
        argv0.clone(),
        "--".into(),
        exe.into(),
    ]);
    Ok(args)
}

/// The inner layer's `PATH`: the wrappers' directory (real tools there),
/// then the dev shell's dirs.
pub fn inner_path(spec: &Spec) -> String {
    std::iter::once(crate::BIN_DIR)
        .chain(spec.inner_path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(":")
}

/// The arguments the real tool gets: git started by the harness itself
/// never runs repository-configured hooks or fsmonitor programs.
pub fn tool_args(exe: &str, args: Vec<OsString>) -> Vec<OsString> {
    if exe != format!("{}/git", crate::BIN_DIR) {
        return args;
    }
    let mut fixed: Vec<OsString> = ["-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null"]
        .map(OsString::from)
        .into();
    fixed.extend(args);
    fixed
}

/// Runs the wrapper for `exe` and returns the exit status: the tool's, or
/// [`SETUP_FAILED`] when the inner layer never started it.
pub fn run(exe: &str, argv0: OsString, args: Vec<OsString>) -> i32 {
    match start(exe, argv0, tool_args(exe, args)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("agent-jail wrapper: {err}");
            SETUP_FAILED
        }
    }
}

fn start(exe: &str, argv0: OsString, args: Vec<OsString>) -> Result<i32, String> {
    use std::io::Read;
    let mut spec = Spec::read()?;
    // The root is read-only, so only existing paths can be masked; a path
    // that does not exist has nothing to hide.
    spec.inner_masks.retain(|mask| Path::new(mask).exists());
    let cwd = std::env::current_dir().unwrap_or_else(|_| crate::WORKSPACE.into());
    let (mut reader, writer) = std::io::pipe().map_err(|err| err.to_string())?;
    let mut bwrap_args = inner_args(&spec, exe, &argv0, &cwd, writer.as_raw_fd())?;
    bwrap_args.extend(args);
    // Single-threaded here, so clearing close-on-exec races nothing.
    rustix::io::fcntl_setfd(&writer, rustix::io::FdFlags::empty())
        .map_err(|err| err.to_string())?;
    let mut child = Command::new(format!("{}/bwrap", crate::JAIL_DIR))
        .args(bwrap_args)
        .spawn()
        .map_err(|err| format!("cannot start the inner layer: {err}"))?;
    drop(writer);
    let mut status_text = String::new();
    let _ = reader.read_to_string(&mut status_text);
    let status = child.wait().map_err(|err| err.to_string())?;
    if !status_text.contains("child-pid") {
        return Err(format!("the inner layer did not start ({status})"));
    }
    Ok(exit_code(status))
}

pub fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Mode;

    fn spec() -> Spec {
        Spec {
            profile: "claude".into(),
            mode: Mode::Harness,
            slots: Default::default(),
            exe: "/bench/x".into(),
            argv: vec![],
            tools: [
                ("/bin/sh", "/nix/store/b/bin/bash"),
                ("/bench/bin/ls", "/nix/store/c/bin/coreutils"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .into(),
            auth_path: "/home/bench/.claude/.credentials.json".into(),
            read_only: vec![],
            policy: Default::default(),
            hooks: vec![],
            inner_masks: vec!["/tmp".into(), "/home/bench".into()],
            dcg: String::new(),
            control: Default::default(),
            store: crate::spec::StoreSpec {
                store: crate::task_env::Store::HostTest,
                exclude: vec![],
                allowed_roots: vec![],
                nix: String::new(),
            },
            inner_path: vec!["/nix/store/r-rust/bin".into()],
            template: vec![],
        }
    }

    #[test]
    fn harness_git_never_runs_repository_hooks() {
        let args = tool_args("/bench/bin/git", vec!["status".into()]);
        assert_eq!(
            args,
            [
                "-c",
                "core.fsmonitor=",
                "-c",
                "core.hooksPath=/dev/null",
                "status"
            ]
            .map(OsString::from)
        );
        assert_eq!(
            tool_args("/bench/bin/ls", vec!["-l".into()]),
            vec![OsString::from("-l")]
        );
    }

    #[test]
    fn inner_args_mask_unshare_and_unwrap() {
        let args = inner_args(
            &spec(),
            "/bin/sh",
            &"sh".into(),
            Path::new("/bench/workspace"),
            9,
        )
        .unwrap();
        let text: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = text.join(" ");
        for needle in [
            "--unshare-net",
            "--unshare-pid",
            "--dev /dev",
            "--bind /bench/jail-private/workspace /bench/workspace --tmpfs",
            "--tmpfs /home/bench",
            "--ro-bind /dev/null /home/bench/.claude/.credentials.json",
            "--unsetenv HTTPS_PROXY",
            "--setenv PATH /bench/bin:/nix/store/r-rust/bin",
            "--ro-bind /nix/store/b/bin/bash /bin/sh",
            "--ro-bind /nix/store/c/bin/coreutils /bench/bin/ls",
            "--argv0 sh -- /bin/sh",
        ] {
            assert!(joined.contains(needle), "{needle} missing from {joined}");
        }
        assert!(
            inner_args(
                &spec(),
                "/usr/bin/python3",
                &"python3".into(),
                Path::new("/"),
                9
            )
            .is_err()
        );
    }
}
