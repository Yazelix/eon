# Checks Eon's managed command PATH and managed shells (Nushell and Bash)
# as packaged, including host mise activation against a fake mise.
{
  pkgs,
  eonPackage,
}:
let
  # A host mise that activates only Bash and Nushell, or fails when asked.
  fakeMise = pkgs.writeShellScriptBin "mise" ''
    if [[ -n ''${FAKE_MISE_FAIL-} ]]; then
      echo 'export EON_MISE_PROBE=failed'
      exit 1
    fi
    case "$*" in
      "activate bash") echo '_mise_hook() { :; }; export EON_MISE_PROBE=bash' ;;
      "activate nu")
        echo 'export-env { $env.EON_MISE_PROBE = "nu" }'
        echo 'def --env mise_hook [] { $env.EON_MISE_HOOK = "ran" }'
        echo 'export def main [] {}'
        ;;
      *) exit 1 ;;
    esac
  '';
in
pkgs.runCommand "eon-managed-command-path-check" { } ''
  export HOME="$TMPDIR" EON_CONFIG_HOME="$TMPDIR/config" PATH=${eonPackage}/libexec/eon/bin
  test "$(${pkgs.coreutils}/bin/readlink -f "$(command -v eon)")" = "$(${pkgs.coreutils}/bin/readlink -f ${eonPackage}/bin/eon)"
  for command in nu bash; do
    version_output=$("$command" --version)
    test -n "$version_output" && test "$("eon-$command" --version)" = "$version_output"
  done
  eon-nu -c 'help clip copy | ignore; help clip paste | ignore'
  test "$(eon-nu --config ${eonPackage}/libexec/eon/shell/nu/0/eon.nu -c 'scope aliases | where name == "clc" | get expansion.0')" = 'clip copy'
  test "$(eon-nu --config ${eonPackage}/libexec/eon/shell/nu/0/eon.nu -c 'scope aliases | where name == "clp" | get expansion.0')" = 'clip paste'
  eon-bash -i -c 'for tool in starship zoxide atuin carapace; do name=_eon_$tool; test -x "''${!name}" || exit 1; done'
  if ${pkgs.gnugrep}/bin/grep -rl ${builtins.storeDir} ${eonPackage}/libexec/eon/shell; then
    exit 1
  elif test $? != 1; then
    exit 1
  fi

  # Each managed shell activates the host's mise, and starts without it
  # when mise is missing, fails, or [shell] mise is false. Nushell's
  # activation is a pre_prompt string hook; the check evaluates it.
  eon_nu=${eonPackage}/libexec/eon/shell/nu/0/eon.nu
  mise_hooks() {
    eon-nu --config "$eon_nu" -c '$env.config.hooks.pre_prompt? | default [] | length'
  }
  no_mise_hooks=$(mise_hooks)
  eon-bash -i -c 'test -z "''${EON_MISE_PROBE-}"'
  PATH="$PATH:${fakeMise}/bin" eon-bash -i -c 'test "$EON_MISE_PROBE" = bash'
  # The hook is due before activation. Its condition turns false once
  # the hook's own `use mise` defines the command mise, or once a
  # sourced activation defines mise_hook.
  PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    let hook = $env.config.hooks.pre_prompt | first
    if not (do $hook.condition) { exit 1 }
    $hook.code | save mise-hook.nu'
  test "$(PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    source mise-hook.nu
    if (do ($env.config.hooks.pre_prompt | first | get condition)) { exit 1 }
    print $"($env.EON_MISE_PROBE) ($env.EON_MISE_HOOK)"')" = 'nu ran'
  PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    def --env mise_hook [] {}
    if (do ($env.config.hooks.pre_prompt | first | get condition)) { exit 1 }'
  export FAKE_MISE_FAIL=1
  PATH="$PATH:${fakeMise}/bin" eon-bash -i -c 'test -z "''${EON_MISE_PROBE-}"'
  test "$(PATH="$PATH:${fakeMise}/bin" mise_hooks)" = "$no_mise_hooks"
  unset FAKE_MISE_FAIL
  ${pkgs.coreutils}/bin/mkdir -p "$EON_CONFIG_HOME"
  printf '[shell]\nmise = false\n' > "$EON_CONFIG_HOME/config.toml"
  PATH="$PATH:${fakeMise}/bin" eon-bash -i -c 'test -z "''${EON_MISE_PROBE-}"'
  test "$(PATH="$PATH:${fakeMise}/bin" mise_hooks)" = "$no_mise_hooks"
  ${pkgs.coreutils}/bin/touch "$out"
''
