use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    thread,
    time::{Duration, Instant},
};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v1.json");
const USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach | versions | config-path";

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
}

fn main() -> ExitCode {
    match execute(env::args_os().skip(1).collect()) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("eon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    match arguments.as_slice() {
        [] if runtime_directory().join("orbit.sock").exists() => attach(),
        [] => run(&[]),
        [command] if command == "run" => run(&[]),
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            run(child)
        }
        [command] if command == "attach" => attach(),
        [command] if command == "versions" => {
            println!(
                "{}",
                eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?
            );
            Ok(0)
        }
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            println!("{}", path.display());
            Ok(0)
        }
        _ => Err(USAGE.into()),
    }
}

fn run(child: &[OsString]) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    supervise(
        &programs(),
        &config,
        &runtime.join("orbit.sock"),
        child,
        Duration::from_secs(5),
    )
}

fn attach() -> Result<i32, String> {
    let runtime = runtime_directory();
    prepare_runtime(&runtime)?;
    let socket = runtime.join("orbit.sock");
    if !socket.exists() {
        return Err(format!(
            "no active Sessions socket at {}; run `eon` first",
            socket.display()
        ));
    }
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    Command::new(programs().venus)
        .arg(socket)
        .env("XDG_CONFIG_HOME", config)
        .status()
        .map(status_code)
        .map_err(|error| format!("cannot launch Eon Desktop: {error}"))
}

fn programs() -> Programs {
    Programs {
        orbit: env::var_os("EON_ORBIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "yazelix-orbit".into()),
        venus: env::var_os("EON_VENUS")
            .map(PathBuf::from)
            .unwrap_or_else(|| "yazelix-venus".into()),
    }
}

fn configuration_directory() -> Result<PathBuf, String> {
    if let Some(path) = nonempty_environment_path("EON_CONFIG_HOME") {
        return Ok(path);
    }
    if let Some(path) = xdg_path(nonempty_environment_path("XDG_CONFIG_HOME")) {
        return Ok(path.join("eon"));
    }
    nonempty_environment_path("HOME")
        .map(|path| path.join(".config/eon"))
        .ok_or_else(|| "HOME, XDG_CONFIG_HOME, and EON_CONFIG_HOME are unset".into())
}

fn prepare_configuration(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

fn runtime_directory() -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join("eon"))
        })
        .unwrap_or_else(|| env::temp_dir().join(format!("eon-{}", effective_uid())))
}

fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn xdg_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| path.is_absolute())
}

fn prepare_runtime(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| {
            format!(
                "cannot create runtime directory {}: {error}",
                path.display()
            )
        })?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect runtime directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "runtime directory {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and no failure state.
    unsafe { libc::geteuid() }
}

fn supervise(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
    socket_timeout: Duration,
) -> Result<i32, String> {
    let previous_socket = socket_identity(socket)?;
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .env("XDG_CONFIG_HOME", config);
    if !child.is_empty() {
        orbit_command.arg("--").args(child);
    }
    let mut orbit = orbit_command
        .spawn()
        .map_err(|error| format!("cannot launch Sessions: {error}"))?;
    if let Err(error) = wait_for_socket(&mut orbit, socket, previous_socket, socket_timeout) {
        stop(&mut orbit);
        return Err(error);
    }

    let mut venus = match Command::new(&programs.venus)
        .arg(socket)
        .env("XDG_CONFIG_HOME", config)
        .spawn()
    {
        Ok(venus) => venus,
        Err(error) => {
            stop(&mut orbit);
            return Err(format!("cannot launch Eon Desktop: {error}"));
        }
    };

    loop {
        if let Some(status) = orbit
            .try_wait()
            .map_err(|error| format!("cannot observe Sessions: {error}"))?
        {
            stop(&mut venus);
            return Ok(status_code(status));
        }
        if venus
            .try_wait()
            .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
            .is_some()
        {
            eprintln!(
                "Eon Desktop exited; Sessions remains active. Run `eon attach` to reconnect."
            );
            return orbit
                .wait()
                .map(status_code)
                .map_err(|error| format!("cannot wait for Sessions: {error}"));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

type SocketIdentity = (u64, u64, i64, i64);

fn socket_identity(socket: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) => Ok(Some((
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect Sessions socket {}: {error}",
            socket.display()
        )),
    }
}

fn wait_for_socket(
    child: &mut Child,
    socket: &Path,
    previous_socket: Option<SocketIdentity>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
        {
            return Err(format!(
                "Sessions exited before creating {} (status {})",
                socket.display(),
                status_code(status)
            ));
        }
        if socket_identity(socket)?.is_some_and(|identity| Some(identity) != previous_socket) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Sessions did not create {} within {} seconds",
                socket.display(),
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{
        Programs, effective_uid, prepare_configuration, prepare_runtime, supervise, xdg_path,
    };
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eon-test-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn executable(path: &Path, source: &str) {
        fs::write(path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn venus_exit_leaves_orbit_and_its_command_running() {
        let root = temporary_directory();
        let orbit = root.join("orbit");
        let venus = root.join("venus");
        let socket = root.join("orbit.sock");
        let config = root.join("config");
        let orbit_log = root.join("orbit.log");
        let venus_log = root.join("venus.log");
        let venus_exited = root.join("venus-exited");
        let stop = root.join("stop");
        fs::write(&socket, "old").unwrap();

        executable(
            &orbit,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$XDG_CONFIG_HOME\" \"$@\" > '{}'\nsleep 0.05\nmv \"$2\" \"$2.old\"\nprintf new > \"$2\"\nwhile test ! -e '{}'; do sleep 0.01; done\nexit 23\n",
                orbit_log.display(),
                stop.display()
            ),
        );
        executable(
            &venus,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$XDG_CONFIG_HOME\" \"$@\" \"$(cat \"$1\")\" > '{}'\n: > '{}'
",
                venus_log.display(),
                venus_exited.display()
            ),
        );

        let stop_after_venus = thread::spawn({
            let venus_exited = venus_exited.clone();
            let stop = stop.clone();
            move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !venus_exited.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(venus_exited.exists());
                thread::sleep(Duration::from_millis(100));
                fs::write(stop, "").unwrap();
            }
        });

        let status = supervise(
            &Programs { orbit, venus },
            &config,
            &socket,
            &["codex".into(), "--model".into(), "test".into()],
            Duration::from_secs(2),
        )
        .unwrap();
        stop_after_venus.join().unwrap();

        assert_eq!(status, 23);
        assert_eq!(
            fs::read_to_string(orbit_log).unwrap(),
            format!(
                "{}\nserve\n{}\n--\ncodex\n--model\ntest\n",
                config.display(),
                socket.display()
            )
        );
        assert_eq!(
            fs::read_to_string(venus_log).unwrap(),
            format!("{}\n{}\nnew\n", config.display(), socket.display())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_directory_is_private() {
        let root = temporary_directory();
        let runtime = root.join("runtime");

        prepare_runtime(&runtime).unwrap();

        let metadata = fs::metadata(&runtime).unwrap();
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(metadata.mode() & 0o777, 0o700);

        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            prepare_runtime(&runtime).unwrap_err(),
            format!(
                "runtime directory {} must be an owned private directory",
                runtime.display()
            )
        );
        assert_eq!(fs::metadata(&runtime).unwrap().mode() & 0o777, 0o777);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_directory_is_private_only_when_created() {
        let root = temporary_directory();
        let config = root.join("config");

        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o700);

        fs::set_permissions(&config, fs::Permissions::from_mode(0o755)).unwrap();
        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o755);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xdg_paths_must_be_absolute() {
        assert_eq!(xdg_path(Some("relative".into())), None);

        let absolute = PathBuf::from("/absolute");
        assert_eq!(xdg_path(Some(absolute.clone())), Some(absolute));
    }
}
