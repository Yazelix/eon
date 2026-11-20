//! Host descriptors. bwrap passes every inherited descriptor without
//! close-on-exec straight into the jail (a heavy.sh or rota `flock`, say),
//! where `/proc/self/fd/N` would reopen a host file, and it has no option to
//! close them. Marking them close-on-exec from Rust needs `unsafe` (rustix
//! has no `close_range`, and `close` and `borrow_raw` are `unsafe`), so
//! `enter` lists them from `/proc` and starts bwrap through the pinned bash
//! with a constant script that closes them. Guard condition 10 fails closed
//! if one is ever missed.

use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;

use crate::Result;

/// `O_CLOEXEC` as `/proc/<pid>/fdinfo` prints it (octal `flags:`).
const O_CLOEXEC: u32 = 0o2_000_000;

/// The launcher's constant script: `{fd}>&-` closes the descriptor named by
/// the variable, with no `eval`, then bash becomes `"$@"`.
pub const CLOSE_SCRIPT: &str = "for fd in $AGENT_JAIL_CLOSE; do exec {fd}>&-; done; exec \"$@\"";

/// Whether an fdinfo text's `flags:` line lacks `O_CLOEXEC`; `None` when
/// the text has no readable `flags:` line.
pub fn lacks_cloexec(fdinfo: &str) -> Option<bool> {
    let flags = fdinfo
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))?;
    let flags = u32::from_str_radix(flags.trim(), 8).ok()?;
    Some(flags & O_CLOEXEC == 0)
}

/// The descriptor numbers in `/proc/self/fd`, sorted.
fn numbers(dir: &Path) -> Result<Vec<i32>> {
    let mut found: Vec<i32> = std::fs::read_dir(dir)
        .map_err(|err| format!("{}: {err}", dir.display()))?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect();
    found.sort_unstable();
    Ok(found)
}

/// The inherited descriptors >= 3 that lack close-on-exec. `enter` calls it
/// first, before it opens anything of its own.
pub fn inherited() -> Result<Vec<i32>> {
    let mut open = Vec::new();
    for fd in numbers(Path::new("/proc/self/fd"))? {
        // The directory listing's own descriptor is gone (and was
        // close-on-exec); a descriptor without fdinfo is closed already.
        let Ok(info) = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")) else {
            continue;
        };
        if fd >= 3 && lacks_cloexec(&info) == Some(true) {
            open.push(fd);
        }
    }
    Ok(open)
}

/// Every open descriptor >= 3 except `allowed` and the one this reads
/// `/proc/self/fd` with (guard condition 10, and the probe).
pub fn open_beyond(allowed: &[i32]) -> Result<Vec<i32>> {
    use rustix::fs::{Mode, OFlags};
    let dir = rustix::fs::open(
        "/proc/self/fd",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| format!("/proc/self/fd: {err}"))?;
    let own = dir.as_raw_fd();
    let mut entries = rustix::fs::Dir::new(dir).map_err(|err| format!("/proc/self/fd: {err}"))?;
    let mut open = Vec::new();
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(|err| format!("/proc/self/fd: {err}"))?;
        if let Some(fd) = entry
            .file_name()
            .to_str()
            .ok()
            .and_then(|name| name.parse::<i32>().ok())
            && fd >= 3
            && fd != own
            && !allowed.contains(&fd)
        {
            open.push(fd);
        }
    }
    open.sort_unstable();
    Ok(open)
}

/// bwrap started through `bash`, which first closes `close`: bash gets an
/// empty environment except `AGENT_JAIL_CLOSE` and reads no startup file, so
/// no host `BASH_ENV` or exported function runs before the loop.
pub fn launcher(bash: &Path, close: &[i32], program: &Path, args: &[String]) -> Command {
    let list: Vec<String> = close
        .iter()
        .filter(|fd| **fd >= 3)
        .map(i32::to_string)
        .collect();
    let mut command = Command::new(bash);
    command
        .env_clear()
        .env("AGENT_JAIL_CLOSE", list.join(" "))
        .args([
            "--norc",
            "--noprofile",
            "-c",
            CLOSE_SCRIPT,
            "agent-jail-close",
        ])
        .arg(program)
        .args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fdinfo_flags_are_read() {
        assert_eq!(
            lacks_cloexec("pos:\t0\nflags:\t02000002\nmnt_id:\t3\n"),
            Some(false)
        );
        assert_eq!(lacks_cloexec("pos:\t0\nflags:\t0100002\n"), Some(true));
        assert_eq!(lacks_cloexec("pos:\t0\n"), None);
        assert_eq!(lacks_cloexec("flags:\tzz\n"), None);
    }

    #[test]
    fn inherited_descriptors_are_listed_and_cloexec_ones_are_not() {
        let (plain, _keep) = std::io::pipe().unwrap();
        rustix::io::fcntl_setfd(&plain, rustix::io::FdFlags::empty()).unwrap();
        let cloexec = std::fs::File::open("/proc/self/status").unwrap();
        let listed = inherited().unwrap();
        assert!(listed.contains(&plain.as_raw_fd()), "{listed:?}");
        assert!(!listed.contains(&cloexec.as_raw_fd()), "{listed:?}");
        assert!(listed.iter().all(|fd| *fd >= 3));
        let open = open_beyond(&[plain.as_raw_fd()]).unwrap();
        assert!(open.contains(&cloexec.as_raw_fd()) && !open.contains(&plain.as_raw_fd()));
    }

    #[test]
    fn the_launcher_runs_a_constant_script_in_a_clean_environment() {
        let command = launcher(
            Path::new("/nix/store/x-bash/bin/bash"),
            &[7, 2, 9],
            Path::new("/nix/store/y-bwrap/bin/bwrap"),
            &["--unshare-net".into()],
        );
        let args: Vec<&std::ffi::OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--norc",
                "--noprofile",
                "-c",
                CLOSE_SCRIPT,
                "agent-jail-close",
                "/nix/store/y-bwrap/bin/bwrap",
                "--unshare-net"
            ]
        );
        let env: Vec<(&std::ffi::OsStr, Option<&std::ffi::OsStr>)> = command.get_envs().collect();
        assert_eq!(env, [("AGENT_JAIL_CLOSE".as_ref(), Some("7 9".as_ref()))]);
        assert!(!CLOSE_SCRIPT.contains("eval"));
    }

    /// Run by `a_hostile_caller_environment_neither_runs_nor_keeps_a_descriptor` in a
    /// child of this test binary whose environment is hostile; idle otherwise.
    #[test]
    #[ignore = "a helper the hostile-environment test runs in a child process"]
    fn hostile_environment_child() {
        let Some(bash) = std::env::var_os("AGENT_JAIL_TEST_BASH") else {
            return;
        };
        let (plain, _keep) = std::io::pipe().unwrap();
        rustix::io::fcntl_setfd(&plain, rustix::io::FdFlags::empty()).unwrap();
        let probe = format!(
            "test -e /proc/self/fd/{} && echo still-open; echo launched-cleanly",
            plain.as_raw_fd()
        );
        let bash = Path::new(&bash);
        let output = launcher(
            bash,
            &inherited().unwrap(),
            bash,
            &["--norc".into(), "-c".into(), probe],
        )
        .output()
        .unwrap();
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    fn a_hostile_caller_environment_neither_runs_nor_keeps_a_descriptor() {
        let dir = crate::test_dir("hostile-env");
        let marker = dir.join("bash-env-ran");
        let script = dir.join("bash-env");
        std::fs::write(&script, format!("echo x > {}\n", marker.display())).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fds::tests::hostile_environment_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("AGENT_JAIL_TEST_BASH", bash_on_path())
            .env("BASH_ENV", &script)
            .env("ENV", &script)
            .env("IFS", "0123456789")
            .env("BASH_FUNC_exec%%", "() { echo hijacked; }")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{text}");
        assert!(text.contains("launched-cleanly"), "{text}");
        assert!(!text.contains("still-open"), "{text}");
        assert!(!text.contains("hijacked"), "{text}");
        assert!(!marker.exists(), "BASH_ENV ran");
    }

    fn bash_on_path() -> std::path::PathBuf {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("bash"))
                    .find(|bash| bash.is_file())
            })
            .expect("no bash on PATH")
    }

    #[test]
    fn the_script_closes_listed_descriptors_and_passes_no_caller_variable() {
        // The child reports which descriptors it still holds and every
        // exported name it got: none of the caller's (PATH, HOME, a BASH_ENV
        // or IFS of the caller) may reach it.
        let (plain, _keep) = std::io::pipe().unwrap();
        rustix::io::fcntl_setfd(&plain, rustix::io::FdFlags::empty()).unwrap();
        let (kept, _also) = std::io::pipe().unwrap();
        rustix::io::fcntl_setfd(&kept, rustix::io::FdFlags::empty()).unwrap();
        let bash = bash_on_path();
        let probe = format!(
            "test -e /proc/self/fd/{} && echo plain-open; test -e /proc/self/fd/{} && echo kept-open; echo env: $(compgen -e)",
            plain.as_raw_fd(),
            kept.as_raw_fd()
        );
        let output = launcher(
            &bash,
            &[plain.as_raw_fd()],
            &bash,
            &["--norc".into(), "-c".into(), probe],
        )
        .output()
        .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("kept-open"), "{text}");
        let names: Vec<&str> = lines
            .next()
            .and_then(|line| line.strip_prefix("env:"))
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        for name in &names {
            assert!(
                ["AGENT_JAIL_CLOSE", "PWD", "OLDPWD", "SHLVL", "_"].contains(name),
                "{name} reached the launched program: {text}"
            );
        }
    }
}
