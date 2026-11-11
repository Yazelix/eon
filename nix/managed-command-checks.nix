# Checks the full product's managed command PATH and shells (Nushell and Bash)
# as packaged, including host mise activation against a fake mise.
{
  pkgs,
  eonPackage,
}:
let
  shellPromptCheck = pkgs.runCommandCC "eon-shell-prompt-check" { nativeBuildInputs = [ pkgs.rustc ]; } ''
    rustc --edition=2024 -D warnings ${./managed-shell-check.rs} -o "$out"
  '';
  starshipBashCheck = pkgs.writeText "eon-starship-check.bash" ''
    set -e
    starship_precmd
    [[ -n $PS1 && $PS1 != $'\n'* ]]
    [[ -z ''${STARSHIP_CONFIG+x} ]]
    if ${pkgs.coreutils}/bin/env | ${pkgs.gnugrep}/bin/grep -q '^STARSHIP_CONFIG='; then exit 1; fi

    printf 'format = "from-home"\n' > "$HOME/.config/starship.toml"
    starship_precmd
    [[ $PS1 == $'\nfrom-home' ]]
    printf 'format = "from-explicit"\n' > "$HOME/explicit starship.toml"
    export STARSHIP_CONFIG="$HOME/explicit starship.toml"
    starship_precmd
    [[ $PS1 == $'\nfrom-explicit' ]]
    STARSHIP_CONFIG=""
    starship_precmd
    [[ $PS1 == $'\n'* ]]
    STARSHIP_CONFIG="$HOME/missing.toml"
    starship_precmd
    [[ $PS1 == $'\n'* ]]

    unset STARSHIP_CONFIG
    ${pkgs.coreutils}/bin/rm "$HOME/.config/starship.toml" "$HOME/explicit starship.toml"
    starship_precmd
    [[ -n $PS1 && $PS1 != $'\n'* ]]
    [[ -z ''${STARSHIP_CONFIG+x} ]]
  '';
  starshipNuCheck = pkgs.writeText "eon-starship-check.nu" ''
    use std/assert
    $env.CMD_DURATION_MS = "0"
    let prompt = do $env.PROMPT_COMMAND
    assert ($prompt | is-not-empty)
    assert (not ($prompt | str starts-with (char newline)))
    assert ('STARSHIP_CONFIG' not-in $env)
    assert (not (^${pkgs.coreutils}/bin/env | lines | any { str starts-with 'STARSHIP_CONFIG=' }))

    let home_config = $env.HOME | path join .config starship.toml
    'format = "from-home"' | save $home_config
    assert equal (do $env.PROMPT_COMMAND) "\nfrom-home"
    let explicit_config = $env.HOME | path join 'explicit starship.toml'
    'format = "from-explicit"' | save $explicit_config
    $env.STARSHIP_CONFIG = $explicit_config
    assert equal (do $env.PROMPT_COMMAND) "\nfrom-explicit"
    $env.STARSHIP_CONFIG = ""
    assert ((do $env.PROMPT_COMMAND) | str starts-with (char newline))
    $env.STARSHIP_CONFIG = ($env.HOME | path join missing.toml)
    assert ((do $env.PROMPT_COMMAND) | str starts-with (char newline))

    hide-env STARSHIP_CONFIG
    rm $home_config $explicit_config
    let prompt = do $env.PROMPT_COMMAND
    assert ($prompt | is-not-empty)
    assert (not ($prompt | str starts-with (char newline)))
    assert ('STARSHIP_CONFIG' not-in $env)
  '';
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
  fail() {
    echo "managed-command-path: $*" >&2
    exit 1
  }
  export HOME="$TMPDIR" EON_CONFIG_HOME="$TMPDIR/config" PATH=${eonPackage}/libexec/eon/bin
  # Model the PATH left by a system bashrc that discarded Runtime's prefix.
  PATH=/usr/bin:/bin ${pkgs.bashInteractive}/bin/bash \
    --rcfile ${eonPackage}/libexec/eon/shell/eon.bashrc -i -c \
    'test "$(command -v eon)" = "${eonPackage}/libexec/eon/shell/../bin/eon" && test "$(command -v hx)" = "${eonPackage}/libexec/eon/shell/../bin/hx"' \
    || fail "managed Bash did not restore Eon's commands after system startup"
  EON_SESSION_BIN=${eonPackage}/libexec/eon/bin \
    PATH=${eonPackage}/libexec/eon/bin:/usr/bin:/bin ${pkgs.bashInteractive}/bin/bash \
    --rcfile ${eonPackage}/libexec/eon/shell/eon.bashrc -i -c '
      expected=$(${pkgs.coreutils}/bin/realpath -m "$EON_SESSION_BIN")
      IFS=: read -ra entries <<< "$PATH"
      count=0
      for entry in "''${entries[@]}"; do
        if [[ $(${pkgs.coreutils}/bin/realpath -m "$entry") == "$expected" ]]; then
          ((count += 1))
        fi
      done
      test "$count" = 1
    ' || fail "managed Bash duplicated the existing session bin"
  # Interactive Bash reads the host's /etc/bashrc before eon.bashrc, and
  # macOS builds have no sandbox to hide it. The user's ~/.bashrc, which
  # eon.bashrc sources before its integrations, undoes what a host file
  # may change: PATH, PROMPT_COMMAND, and integrations already set up.
  printf '%s\n' \
    'PATH=$EON_CHECK_PATH' \
    'unset PROMPT_COMMAND' \
    'unset -f starship_precmd __zoxide_hook __atuin_history _carapace_completer _mise_hook' \
    'if [[ ''${EON_CHECK_CUSTOM_PROMPT-} == 1 ]]; then PS1=custom; fi' \
    > "$HOME/.bashrc"
  # Runs COMMAND in managed Bash with PATH set to SEARCH_PATH.
  managed_bash() {
    EON_CHECK_PATH=$1 PATH=$1 eon-bash -i -c "$2"
  }
  test "$(${pkgs.coreutils}/bin/readlink -f "$(command -v eon)")" = "$(${pkgs.coreutils}/bin/readlink -f ${eonPackage}/bin/eon)" \
    || fail "eon on the managed PATH is not the package's eon"
  for command in nu bash; do
    version_output=$("$command" --version) || fail "$command --version failed"
    test -n "$version_output" && test "$("eon-$command" --version)" = "$version_output" \
      || fail "eon-$command --version differs from $command --version"
  done
  eon-nu -c 'help clip copy | ignore; help clip paste | ignore' \
    || fail "Nushell lacks clip copy or clip paste"
  test "$(eon-nu --config ${eonPackage}/libexec/eon/shell/nu/0/eon.nu -c 'scope aliases | where name == "clc" | get expansion.0')" = 'clip copy' \
    || fail "Nushell alias clc is not clip copy"
  test "$(eon-nu --config ${eonPackage}/libexec/eon/shell/nu/0/eon.nu -c 'scope aliases | where name == "clp" | get expansion.0')" = 'clip paste' \
    || fail "Nushell alias clp is not clip paste"
  # A scoped overlay leaves directory completion's working directory stale.
  eon-nu --config ${eonPackage}/libexec/eon/shell/nu/15/eon.nu -c '
    use std/assert
    assert equal (overlay list | get name) [zero]
    assert equal $env.STARSHIP_SHELL nu
    assert equal ($env.PROMPT_COMMAND | describe) closure' \
    || fail "Nushell Starship activation changes overlays or loses its prompt"
  managed_bash "$PATH" 'for tool in starship zoxide atuin carapace; do name=_eon_$tool; test -x "''${!name}" || exit 1; done' \
    || fail "managed Bash did not set up Starship, Zoxide, Atuin and Carapace"

  # Exercise shell-owned prompts, including configs created after startup.
  # XDG_CONFIG_HOME deliberately differs: Starship discovers only ~/.config.
  (
    unset STARSHIP_CONFIG
    export XDG_CONFIG_HOME="$TMPDIR/starship-xdg"
    ${pkgs.coreutils}/bin/mkdir -p "$HOME/.config" "$XDG_CONFIG_HOME" "$EON_CONFIG_HOME"
    printf 'format = "from-xdg"\n' > "$XDG_CONFIG_HOME/starship.toml"
    managed_bash "$PATH" 'source ${starshipBashCheck}' \
      || fail "managed Bash lost Starship default or user-config precedence"
    eon-nu --config ${eonPackage}/libexec/eon/shell/nu/1/eon.nu -c 'source ${starshipNuCheck}' \
      || fail "managed Nushell lost Starship default or user-config precedence"

    printf '[shell]\nstarship = false\n' > "$EON_CONFIG_HOME/config.toml"
    managed_bash "$PATH" '! declare -F _eon_starship_prompt >/dev/null' \
      || fail "managed Bash applied its Starship default while disabled"
    eon-nu --config ${eonPackage}/libexec/eon/shell/nu/0/eon.nu -c \
      'if (scope commands | any { $in.name == "_eon_starship_prompt" }) { exit 1 }' \
      || fail "managed Nushell applied its Starship default while disabled"
    ${pkgs.coreutils}/bin/rm "$EON_CONFIG_HOME/config.toml"
    EON_CHECK_CUSTOM_PROMPT=1 managed_bash "$PATH" \
      '[[ $PS1 == custom ]] && ! declare -F _eon_starship_prompt >/dev/null' \
      || fail "managed Bash replaced a custom prompt"
    printf '%s\n' '$env.PROMPT_COMMAND = {|| "custom" }' \
      'source ${eonPackage}/libexec/eon/shell/nu/1/eon.nu' > "$TMPDIR/custom-prompt.nu"
    eon-nu --config "$TMPDIR/custom-prompt.nu" -c \
      'use std/assert; assert equal (do $env.PROMPT_COMMAND) custom' \
      || fail "managed Nushell replaced a custom prompt"
  )
  if ${pkgs.gnugrep}/bin/grep -rl ${builtins.storeDir} ${eonPackage}/libexec/eon/shell; then
    fail "the shell tree names store paths"
  elif test $? != 1; then
    fail "cannot search the shell tree"
  fi

  # Each managed shell activates the host's mise, and starts without it
  # when mise is missing, fails, or [shell] mise is false. Nushell's
  # activation is a pre_prompt string hook; the check evaluates it.
  eon_nu=${eonPackage}/libexec/eon/shell/nu/0/eon.nu
  mise_hooks() {
    eon-nu --config "$eon_nu" -c '$env.config.hooks.pre_prompt? | default [] | length'
  }
  no_mise_hooks=$(mise_hooks) || fail "Nushell cannot load eon.nu"
  managed_bash "$PATH" 'test -z "''${EON_MISE_PROBE-}"' \
    || fail "managed Bash activated mise with none on PATH"
  managed_bash "$PATH:${fakeMise}/bin" 'test "$EON_MISE_PROBE" = bash' \
    || fail "managed Bash did not activate the mise on PATH"
  # The hook is due before activation. Its condition turns false once
  # the hook's own `use mise` defines the command mise, or once a
  # sourced activation defines mise_hook.
  PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    let hook = $env.config.hooks.pre_prompt | first
    if not (do $hook.condition) { exit 1 }
    $hook.code | save mise-hook.nu' \
    || fail "Nushell's mise hook is not due before activation"
  test "$(PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    source mise-hook.nu
    if (do ($env.config.hooks.pre_prompt | first | get condition)) { exit 1 }
    print $"($env.EON_MISE_PROBE) ($env.EON_MISE_HOOK)"')" = 'nu ran' \
    || fail "Nushell's mise hook did not activate mise, or stayed due after it"
  PATH="$PATH:${fakeMise}/bin" eon-nu --config "$eon_nu" -c '
    def --env mise_hook [] {}
    if (do ($env.config.hooks.pre_prompt | first | get condition)) { exit 1 }' \
    || fail "Nushell's mise hook stays due after a sourced activation"
  export FAKE_MISE_FAIL=1
  managed_bash "$PATH:${fakeMise}/bin" 'test -z "''${EON_MISE_PROBE-}"' \
    || fail "managed Bash kept a failed mise activation"
  test "$(PATH="$PATH:${fakeMise}/bin" mise_hooks)" = "$no_mise_hooks" \
    || fail "Nushell added a mise hook when mise fails"
  unset FAKE_MISE_FAIL
  ${pkgs.coreutils}/bin/mkdir -p "$EON_CONFIG_HOME"
  printf '[shell]\nmise = false\n' > "$EON_CONFIG_HOME/config.toml"
  managed_bash "$PATH:${fakeMise}/bin" 'test -z "''${EON_MISE_PROBE-}"' \
    || fail "managed Bash activated mise with [shell] mise = false"
  test "$(PATH="$PATH:${fakeMise}/bin" mise_hooks)" = "$no_mise_hooks" \
    || fail "Nushell added a mise hook with [shell] mise = false"
  ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
    ${shellPromptCheck} ${eonPackage} ${pkgs.util-linux}/bin/script \
      ${pkgs.bash-preexec}/share/bash/bash-preexec.sh ${pkgs.blesh}/share/blesh/ble.sh \
      ${pkgs.lib.makeBinPath [ pkgs.coreutils pkgs.gnused pkgs.gnugrep pkgs.gawk pkgs.ncurses pkgs.procps pkgs.bashInteractive ]} \
      ${pkgs.bash}/bin/bash

    # Eon's Helix is built without Steel (EON-C9): hx runs no init.scm, from a
    # repository's .helix or from Eon's Helix directory. Helix needs a
    # terminal, which script gives it. A Steel build runs init.scm before
    # its first frame, so the check waits until Helix has drawn the file's
    # name.
    ${pkgs.coreutils}/bin/mkdir -p "$TMPDIR/repo/.helix" "$EON_CONFIG_HOME/helix"
    : > "$TMPDIR/repo/.helix/helix.scm"
    : > "$EON_CONFIG_HOME/helix/helix.scm"
    printf '(create-directory! "%s")\n' "$TMPDIR/repo-steel" > "$TMPDIR/repo/.helix/init.scm"
    printf '(create-directory! "%s")\n' "$TMPDIR/user-steel" > "$EON_CONFIG_HOME/helix/init.scm"
    (cd "$TMPDIR/repo" && TERM=xterm-256color exec ${pkgs.util-linux}/bin/script -qec 'hx notes.txt' /dev/null) \
      < /dev/null > "$TMPDIR/helix.log" 2>&1 &
    helix=$!
    drawn=
    for _ in $(${pkgs.coreutils}/bin/seq 600); do
      if ${pkgs.gnugrep}/bin/grep -aq 'notes\.txt' "$TMPDIR/helix.log"; then
        drawn=1
        break
      fi
      ${pkgs.coreutils}/bin/sleep 0.1
    done
    kill "$helix" 2> /dev/null || true
    wait "$helix" || true
    if test -z "$drawn" || test -e "$TMPDIR/repo-steel" || test -e "$TMPDIR/user-steel"; then
      ${pkgs.coreutils}/bin/tail -c 4096 "$TMPDIR/helix.log" | ${pkgs.coreutils}/bin/cat -v >&2
      test -n "$drawn" || fail "Helix did not draw notes.txt"
      test ! -e "$TMPDIR/repo-steel" \
        || fail "Helix ran the repository's .helix/init.scm"
      fail "Helix ran init.scm from Eon's Helix directory"
    fi
  ''}
  ${pkgs.coreutils}/bin/touch "$out"
''
