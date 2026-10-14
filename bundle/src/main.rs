use std::{env, fs, io, os::unix::process::CommandExt, process::Command};

fn launch() -> io::Result<()> {
    let executable = env::current_exe()?;
    let bundle = executable
        .parent()
        .ok_or_else(|| io::Error::other("cannot locate bundle directory"))?;
    let mut command = Command::new(bundle.join("libexec/bwrap"));
    command.args([
        "--unshare-user",
        "--dev-bind",
        "/dev",
        "/dev",
        "--proc",
        "/proc",
    ]);
    // Build a fresh root so hosts without /nix need no privileged directory creation.
    for entry in fs::read_dir("/")? {
        let entry = entry?;
        if ["nix", "dev", "proc"]
            .iter()
            .any(|name| entry.file_name() == *name)
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            command
                .arg("--symlink")
                .arg(fs::read_link(&path)?)
                .arg(path);
        } else {
            command.arg("--bind").arg(&path).arg(path);
        }
    }
    command
        .arg("--ro-bind")
        .arg(bundle.join("nix"))
        .arg("/nix")
        .arg("--")
        .arg(env!("EON_BUNDLE_PROGRAM"))
        .args(env::args_os().skip(1));
    Err(command.exec())
}

fn main() {
    if let Err(error) = launch() {
        eprintln!("eon bundle: {error}");
        std::process::exit(1);
    }
}
