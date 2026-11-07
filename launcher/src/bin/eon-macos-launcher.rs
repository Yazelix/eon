//! LaunchServices adapter for the Nix app. Runtime still owns the managed environment.
use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStringExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// A safety bound: a broken login startup must not prevent the app from opening.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PATH_BYTES: u64 = 64 * 1024;

unsafe extern "C" {
    fn getuid() -> u32;
    fn kill(pid: i32, signal: i32) -> i32;
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("eon-app-{}-{stamp}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Only the dedicated file carries the value; failed startup kills its private group.
fn run(command: &mut Command, deadline: Instant) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn()?;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("login command exited {status}")))
                };
            }
            Err(error) => break Err(error),
            Ok(None) if Instant::now() >= deadline => {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "login PATH timed out",
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    };
    if result.is_err() {
        // SAFETY: this group was created for this child alone; it never joins Eon's group.
        unsafe { kill(-(child.id() as i32), 9) };
    }
    let _ = child.wait();
    result
}

fn read_value(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_PATH_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PATH_BYTES || bytes.contains(&0) {
        return Err(io::Error::other("invalid or oversized login PATH"));
    }
    Ok(bytes)
}

fn account_shell(scratch: &Path, deadline: Instant) -> io::Result<PathBuf> {
    let output = scratch.join("account");
    // SAFETY: getuid has no arguments and no failure case.
    let uid = unsafe { getuid() };
    run(
        Command::new("/usr/bin/dscacheutil")
            .args(["-q", "user", "-a", "uid", &uid.to_string()])
            .stdout(File::create(&output)?),
        deadline,
    )?;
    let record = read_value(&output)?;
    record
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"shell: "))
        .map(|shell| PathBuf::from(OsString::from_vec(shell.to_vec())))
        .filter(|shell| shell.is_absolute())
        .ok_or_else(|| io::Error::other("account database has no absolute login shell"))
}

fn path_command(
    shell: &Path,
    output: &Path,
    printenv: &str,
) -> io::Result<(&'static [&'static str], String)> {
    let name = shell.file_name().and_then(|name| name.to_str());
    let arguments: &[&str] = match name {
        Some("nu") => &["--login", "--interactive", "--commands"],
        Some("bash" | "zsh") => &["-l", "-i", "-c"],
        _ => return Err(io::Error::other("unsupported account login shell")),
    };
    // The private filename is quoted as data; never evaluate startup output.
    let quoted = output
        .to_str()
        .ok_or_else(|| io::Error::other("non-UTF-8 temporary path"))?;
    if name == Some("nu") && quoted.contains('\'') {
        return Err(io::Error::other(
            "Nushell response path contains an apostrophe",
        ));
    }
    let script = if name == Some("nu") {
        format!("^{printenv} PATH out> '{quoted}'")
    } else {
        format!("{printenv} PATH > '{}'", quoted.replace('\'', "'\\''"))
    };
    Ok((arguments, script))
}

fn login_path(shell: &Path, scratch: &Path, deadline: Instant) -> io::Result<OsString> {
    if deadline.saturating_duration_since(Instant::now()) < Duration::from_millis(100) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "no time left for login PATH",
        ));
    }
    let output = scratch.join("path");
    let (arguments, script) = path_command(shell, &output, "/usr/bin/printenv")?;
    run(
        Command::new(shell)
            .args(arguments)
            .arg(script)
            .stdout(Stdio::null()),
        deadline,
    )?;
    let mut bytes = read_value(&output)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Err(io::Error::other("login shell returned an empty PATH"));
    }
    Ok(OsString::from_vec(bytes))
}

fn append_diagnostic(directory: &Path, message: &str) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    // SAFETY: getuid has no arguments and no failure case.
    let uid = unsafe { getuid() };
    if !metadata.is_dir() || metadata.uid() != uid {
        return Err(io::Error::other("app log directory must be owned"));
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let path = directory.join("app-launch.log");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.uid() != uid => {
            return Err(io::Error::other("app log must be an owned regular file"));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut file = File::options()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    writeln!(file, "eon app: {message}")
}

fn diagnostic(message: &str) {
    eprintln!("eon app: {message}");
    let absolute = |name| {
        env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let directory = absolute("XDG_STATE_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local/state")))
        .map(|state| state.join("eon/logs"));
    if let Some(directory) = directory
        && let Err(error) = append_diagnostic(&directory, message)
    {
        eprintln!("eon app: cannot write app-launch.log: {error}");
    }
}

fn launch() -> io::Result<()> {
    let resources = env::current_exe()?.parent().unwrap().join("../Resources");
    let command = PathBuf::from(OsString::from_vec(fs::read(resources.join("eon-command"))?));
    if !command.is_absolute() {
        return Err(io::Error::other("app has no absolute Eon launch input"));
    }
    let imported = (|| {
        let scratch = Scratch::new()?;
        let deadline = Instant::now() + LOGIN_TIMEOUT;
        let shell = account_shell(&scratch.0, deadline)?;
        login_path(&shell, &scratch.0, deadline)
    })();
    let path = match imported {
        Ok(path) => path,
        Err(error) => {
            diagnostic(&format!("{error}; keeping the launch PATH"));
            env::var_os("PATH").unwrap_or_default()
        }
    };
    let package = command.parent().unwrap().parent().unwrap();
    let path = env::join_paths(
        [package.join("libexec/eon/bin"), package.join("bin")]
            .into_iter()
            .chain(env::split_paths(&path)),
    )
    .map_err(io::Error::other)?;
    let mut child = Command::new(command);
    child.args(env::args_os().skip(1)).env("PATH", path);
    Err(child.exec())
}

fn main() {
    if let Err(error) = launch() {
        diagnostic(&error.to_string());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_use_the_account_shells_syntax() {
        for name in ["bash", "zsh"] {
            let (arguments, script) =
                path_command(Path::new(name), Path::new("/tmp/a'b"), "/usr/bin/printenv").unwrap();
            assert_eq!(arguments, ["-l", "-i", "-c"]);
            assert_eq!(script, "/usr/bin/printenv PATH > '/tmp/a'\\''b'");
        }
        let (arguments, script) =
            path_command(Path::new("nu"), Path::new("/tmp/a b"), "/usr/bin/printenv").unwrap();
        assert_eq!(arguments, ["--login", "--interactive", "--commands"]);
        assert_eq!(script, "^/usr/bin/printenv PATH out> '/tmp/a b'");
        assert!(path_command(Path::new("nu"), Path::new("/tmp/a'b"), "/usr/bin/printenv").is_err());
        assert!(path_command(Path::new("ksh"), Path::new("/tmp/a"), "/usr/bin/printenv").is_err());
    }

    #[test]
    fn real_shells_write_the_path_response() {
        let scratch = Scratch::new().unwrap();
        let locate = |name| {
            env::split_paths(&env::var_os("PATH").unwrap())
                .map(|directory| directory.join(name))
                .find(|path| path.is_file())
                .expect("shell and printenv on the test PATH")
        };
        let printenv = locate("printenv");
        for name in ["bash", "nu"] {
            let output = scratch.0.join(format!("response with spaces-{name}"));
            let (arguments, script) =
                path_command(Path::new(name), &output, printenv.to_str().unwrap()).unwrap();
            let mut command = Command::new(locate(name));
            command.args(if name == "nu" {
                &["--no-config-file"][..]
            } else {
                &["--noprofile", "--norc"][..]
            });
            run(
                command
                    .args(arguments)
                    .arg(script)
                    .env("PATH", "/profile tools:/bin")
                    .stdout(Stdio::null()),
                Instant::now() + LOGIN_TIMEOUT,
            )
            .unwrap();
            assert_eq!(
                read_value(&output).unwrap(),
                b"/profile tools:/bin\n",
                "{name}"
            );
        }
    }

    #[test]
    fn diagnostics_append_privately_and_reject_a_symlink() {
        let scratch = Scratch::new().unwrap();
        let logs = scratch.0.join("logs");
        append_diagnostic(&logs, "one").unwrap();
        append_diagnostic(&logs, "two").unwrap();
        let path = logs.join("app-launch.log");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "eon app: one\neon app: two\n"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(scratch.0.join("other"), &path).unwrap();
        assert!(append_diagnostic(&logs, "no").is_err());
        assert!(!scratch.0.join("other").exists());
    }

    #[test]
    fn successful_startup_keeps_its_background_work_alive() {
        let scratch = Scratch::new().unwrap();
        let shell = scratch.0.join("bash");
        let done = scratch.0.join("done");
        fs::write(
            &shell,
            format!(
                "#!/bin/sh\n(sleep 0.1; printf done > '{}') &\nprintf '/bin\\n' > '{}'\n",
                done.display(),
                scratch.0.join("path").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();
        login_path(&shell, &scratch.0, Instant::now() + LOGIN_TIMEOUT).unwrap();
        let deadline = Instant::now() + LOGIN_TIMEOUT;
        while !done.is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(done).unwrap(), "done");
    }

    #[test]
    fn login_import_discards_noise_preserves_path_and_bounds_failure() {
        let scratch = Scratch::new().unwrap();
        let shell = scratch.0.join("zsh");
        fs::write(&shell, format!("#!/bin/sh\necho noise\necho noise >&2\ntest \"$1 $2 $3\" = '-l -i -c' || exit 1\nprintf '/my tools:/bin\\n' > '{}'\n", scratch.0.join("path").display())).unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            login_path(&shell, &scratch.0, Instant::now() + LOGIN_TIMEOUT).unwrap(),
            "/my tools:/bin"
        );
        fs::write(&shell, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(login_path(&shell, &scratch.0, Instant::now() + LOGIN_TIMEOUT).is_err());
        let started = scratch.0.join("started");
        fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf started > '{}'\nexec sleep 30\n",
                started.display()
            ),
        )
        .unwrap();
        let start = Instant::now();
        assert_eq!(
            login_path(&shell, &scratch.0, start + Duration::from_millis(30))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(
            !started.exists(),
            "the nearly expired deadline started a shell"
        );
        let start = Instant::now();
        assert_eq!(
            run(
                Command::new(&shell).stdout(Stdio::null()),
                start + Duration::from_millis(30)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::TimedOut
        );
    }
}
