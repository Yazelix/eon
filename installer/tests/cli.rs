use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn empty_and_relative_xdg_use_the_same_fallback_as_unset() {
    let home = tempfile::tempdir().unwrap();
    let run = |data: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_eon-install"));
        command
            .args(["remove", "--dry-run"])
            .env("HOME", home.path());
        if let Some(data) = data {
            command.env("XDG_DATA_HOME", data);
        } else {
            command.env_remove("XDG_DATA_HOME");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{:?}", output);
        output.stdout
    };
    let baseline = run(None);
    assert_eq!(run(Some("")), baseline);
    assert_eq!(run(Some("relative/path")), baseline);
    assert!(home.path().read_dir().unwrap().next().is_none());
}

#[test]
fn missing_and_old_ssh_keygen_report_the_host_requirement_before_download() {
    let temp = tempfile::tempdir().unwrap();
    let signers = temp.path().join("allowed_signers");
    fs::write(&signers, "").unwrap();
    let prefix = temp.path().join("prefix");
    let run = || {
        let output = Command::new(env!("CARGO_BIN_EXE_eon-install"))
            .args([
                "install",
                "https://proof.invalid/eon-0.1.21-x86_64-linux.tar.zst",
                "--signers",
            ])
            .arg(&signers)
            .arg("--prefix")
            .arg(&prefix)
            .env("PATH", temp.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(!prefix.exists());
        String::from_utf8(output.stderr).unwrap()
    };
    assert!(run().contains("ssh-keygen missing"));
    let shell = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    assert!(shell.status.success());
    let script = format!(
        "#!{}\nprintf '%s\\n' 'Unsupported operation for -Y: \"match-principals\"' >&2\nexit 1\n",
        String::from_utf8(shell.stdout).unwrap().trim()
    );
    let keygen = temp.path().join("ssh-keygen");
    assert!(
        Command::new("sh")
            .args(["-c", "printf '%s' \"$1\" > \"$0\""])
            .arg(&keygen)
            .arg(script)
            .status()
            .unwrap()
            .success()
    );
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(run().contains("this ssh-keygen is too old (needs OpenSSH 8.9+)"));
}
