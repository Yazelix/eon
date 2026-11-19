# Workflows and shell scripts: actionlint over every workflow (with ShellCheck on
# its run: blocks), ShellCheck over every *.sh and *.bash file, and no workflow trigger that hands pull request code the repository's secrets
# (docs/DISTRIBUTION.md, Pull request checks).
{ pkgs }:
let
  inherit (pkgs) lib;
  source = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../.github
      (lib.fileset.fileFilter (file: lib.any file.hasExt [ "sh" "bash" ]) ../.)
    ];
  };
in
pkgs.runCommand "eon-lint" { nativeBuildInputs = [ pkgs.actionlint pkgs.shellcheck ]; } ''
  cd ${source}
  shopt -s nullglob
  workflows=(.github/workflows/*.yml .github/workflows/*.yaml)
  test "''${#workflows[@]}" -gt 0
  actionlint "''${workflows[@]}"
  mapfile -d "" scripts < <(find . -type f \( -name '*.sh' -o -name '*.bash' \) -print0 | sort -z)
  test "''${#scripts[@]}" -gt 0
  shellcheck "''${scripts[@]}"
  # Both triggers run with the base repository's secrets and a write token.
  if grep -nwE 'pull_request_target|workflow_run' "''${workflows[@]}"; then
    echo "a workflow names pull_request_target or workflow_run; pull requests use pull_request only" >&2
    exit 1
  elif test $? != 1; then
    exit 1
  fi
  touch "$out"
''
