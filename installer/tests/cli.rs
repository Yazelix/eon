use std::process::Command;

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
