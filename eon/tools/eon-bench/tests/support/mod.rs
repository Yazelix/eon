use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

pub fn bind_fixture(command: &mut Command, root: &Path) {
    let wrapper = root.join("fixture-bwrap");
    fs::write(
        &wrapper,
        format!(
            "#!{}\n{}",
            super::executable("bash").display(),
            r#"
args=()
while [ "$#" -gt 0 ] && [ "$1" != -- ]; do
    args+=("$1")
    shift
done
[ "$1" = -- ] || exit 2
shift
exec "$FIXTURE_BWRAP" "${args[@]}" --bind "$FIXTURE_ROOT" "$FIXTURE_ROOT" -- "$@"
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    // Fake drivers and their oracles are explicit inputs, without exposing the rest of home.
    command
        .env("EON_BENCH_BWRAP", wrapper)
        .env("FIXTURE_BWRAP", super::executable("bwrap"))
        .env("FIXTURE_ROOT", root);
}
