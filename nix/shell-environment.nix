# Startup files for the managed shells. Eon's Nix composition passes the
# exact pinned tools and installs the returned tree at libexec/eon/shell, next
# to the session bin. Each file names managed tools and its sibling files
# relative to its own location, so the tree holds no absolute store path and
# its paths become opaque launch inputs.
{ pkgs, starshipPackage, zoxidePackage, atuinPackage, carapacePackage }:
let
  # Files land in init/, so the session bin is ../../bin from each of them.
  shellInit = pkgs.runCommand "eon-shell-init" { } ''
    mkdir -p "$out"
    # Keep native config commands writable and notice configs created mid-session.
    cat > "$out/starship-nu" <<'NU'
    const _eon_starship_config = path self ../starship.toml
    def --wrapped _eon_starship_prompt [...args] {
      let config = if ('STARSHIP_CONFIG' not-in $env) and not ($env.HOME | path join .config starship.toml | path exists) {
        {STARSHIP_CONFIG: $_eon_starship_config}
      } else { {} }
      with-env $config { ^$_eon_starship ...$args }
    }
    NU
    ${starshipPackage}/bin/starship init nu >> "$out/starship-nu"
    cat > "$out/starship-bash" <<'BASH'
    _eon_starship_prompt() {
      if [[ -z ''${STARSHIP_CONFIG+x} && ! -e $HOME/.config/starship.toml ]]; then
        STARSHIP_CONFIG="''${BASH_SOURCE[0]%/*}/../starship.toml" "$_eon_starship" "$@"
      else
        "$_eon_starship" "$@"
      fi
    }
    BASH
    ${starshipPackage}/bin/starship init bash --print-full-init >> "$out/starship-bash"
    sed -i 's#\^${starshipPackage}/bin/starship#_eon_starship_prompt#g' "$out/starship-nu"
    sed -i 's#${starshipPackage}/bin/starship prompt#_eon_starship_prompt prompt#g' "$out/starship-bash"
    sed -i 's#${starshipPackage}/bin/starship#"$_eon_starship"#g' "$out/starship-bash"

    for shell in nushell bash; do
      ${zoxidePackage}/bin/zoxide init "$shell" > "$out/zoxide-$shell"
    done
    substituteInPlace "$out/zoxide-nushell" \
      --replace-fail '^zoxide ' '^$_eon_zoxide '
    substituteInPlace "$out/zoxide-bash" \
      --replace-fail '\command zoxide ' '\command "$_eon_zoxide" '

    mkdir -p "$TMPDIR/home" "$TMPDIR/config"
    for shell in nu bash; do
      HOME="$TMPDIR/home" XDG_CONFIG_HOME="$TMPDIR/config" \
        ${atuinPackage}/bin/atuin init "$shell" --disable-up-arrow --disable-ai \
        > "$out/atuin-$shell"
      ATUIN_NOBIND=1 HOME="$TMPDIR/home" XDG_CONFIG_HOME="$TMPDIR/config" \
        ${atuinPackage}/bin/atuin init "$shell" --disable-up-arrow --disable-ai \
        > "$out/atuin-$shell-nobind"
    done
    for file in "$out"/atuin-nu "$out"/atuin-nu-nobind; do
      # Keybinding commands run later in another scope, so embed the path.
      substituteInPlace "$file" \
        --replace-fail "'let output = (run-external atuin search'" \
          '$"let output = \(run-external ($_eon_atuin | to nuon) search"' \
        --replace-fail "'(run-external atuin search'" \
          '$"\(run-external ($_eon_atuin | to nuon) search"'
      sed -E -i 's#\^?atuin (uuid|history|search)#^$_eon_atuin \1#g' "$file"
    done
    for file in "$out"/atuin-bash "$out"/atuin-bash-nobind; do
      # The tmux popup reparses this string with sh.
      substituteInPlace "$file" \
        --replace-fail "ATUIN_QUERY='\$escaped_query' atuin search" \
          "ATUIN_QUERY='\$escaped_query' \''${_eon_atuin@Q} search"
      sed -E -i 's#atuin (uuid|history|search)#"$_eon_atuin" \1#g' "$file"
    done

    for shell in nushell bash; do
      HOME="$TMPDIR/home" XDG_CONFIG_HOME=/eon-carapace-config \
        ${carapacePackage}/bin/carapace _carapace "$shell" \
        > "$out/carapace-$shell"
    done
    sed -i \
      '1c export PATH="''${XDG_CONFIG_HOME:-$HOME/.config}/carapace/bin:$PATH"' \
      "$out/carapace-bash"
    sed -i \
      's|xargs carapace |xargs "$_eon_carapace" |g' \
      "$out/carapace-bash"
    tail -n +2 "$out/carapace-nushell" > "$TMPDIR/carapace-nushell"
    printf '%s\n' '$env.PATH = ($env.PATH | split row (char esep) | where { $in != (($env.XDG_CONFIG_HOME? | default ($env.HOME | path join ".config")) | path join "carapace" "bin") } | prepend (($env.XDG_CONFIG_HOME? | default ($env.HOME | path join ".config")) | path join "carapace" "bin"))' \
      > "$out/carapace-nushell"
    sed \
      's|  carapace $spans.0 nushell|  ^$_eon_carapace $spans.0 nushell|' \
      "$TMPDIR/carapace-nushell" >> "$out/carapace-nushell"

    for file in "$out"/*; do
      tool=''${file##*/}
      tool=''${tool%%-*}
      case $file in
        *-bash | *-bash-nobind)
          sed -i "1i _eon_$tool=\''${BASH_SOURCE[0]%/*}/../../bin/$tool" "$file"
          ;;
        *)
          sed -i "1i const _eon_$tool = path self ../../bin/$tool" "$file"
          ;;
      esac
    done
  '';
  # Autoload files land in nu/<mask>/, so init/ is ../../init from each.
  nuBase = pkgs.writeText "eon-nu-base" ''
    const _eon_shell_init = path self ../../init
    $env.config.show_banner = false
    $env.config.shell_integration.osc633 = false
    $env.config.hooks.pre_prompt = ($env.config.hooks.pre_prompt? | default [] | append {||
      if not (is-terminal --stdout) { $env.config.shell_integration.osc133 = false }
    })
    alias clc = clip copy
    alias clp = clip paste
  '';
  nuStarship = pkgs.writeText "eon-nu-starship" ''
    let eon_prompt_is_default = {|prompt|
      match ($prompt | describe) {
        "nothing" => true
        "closure" => ((view source $prompt | metadata).source == "default_env.nu")
        _ => false
      }
    }
    if (
      (do $eon_prompt_is_default $env.PROMPT_COMMAND?) and
      (do $eon_prompt_is_default $env.PROMPT_COMMAND_RIGHT?)
    ) {
      try {
        source ($_eon_shell_init | path join starship-nu)
      } catch {|error|
        print --stderr $"eon: cannot initialize Starship: ($error.msg)"
      }
    }
  '';
  nuZoxide = pkgs.writeText "eon-nu-zoxide" ''
    source ($_eon_shell_init | path join zoxide-nushell)
  '';
  nuAtuinSnippet =
    name: initializer:
    pkgs.writeText name ''
      if not (
        (scope commands | any {|command| $command.name == "_atuin_search_cmd" }) or
        ($env.config.keybindings? | default [] | any {|binding| ($binding.name? | default "") == "atuin" })
      ) {
        try {
          source ($_eon_shell_init | path join ${initializer})
        } catch {|error|
          print --stderr $"eon: cannot initialize Atuin: ($error.msg)"
        }
      }
    '';
  nuAtuin = nuAtuinSnippet "eon-nu-atuin" "atuin-nu";
  nuAtuinNoBind = nuAtuinSnippet "eon-nu-atuin-nobind" "atuin-nu-nobind";
  nuCarapace = pkgs.writeText "eon-nu-carapace" ''
    if (($env.config.completions.external.completer? | default null) == null) {
      try {
        source ($_eon_shell_init | path join carapace-nushell)
      } catch {|error|
        print --stderr $"eon: cannot initialize Carapace: ($error.msg)"
      }
    }
  '';
  # Host mise, activated as `mise activate nu` prints it. That output is a
  # module, which Nu loads only from a path known at parse time, so a one-shot
  # string hook, parsed when it runs, loads it before the first prompt. The
  # added command runs mise's hook once, so the first command already sees
  # the directory's tools; mise's own hooks take over from there.
  nuMise = pkgs.writeText "eon-nu-mise" ''
    # A used activation defines the command mise; a sourced one, mise_hook.
    let eon_mise_inactive = {|| which mise mise_hook | where type == custom | is-empty }
    if ($env.EON_SHELL_MISE? == "1") and (do $eon_mise_inactive) {
      let activation = try { ^mise activate nu | complete } catch { null }
      if ($activation != null) and ($activation.exit_code == 0) and ($activation.stdout | is-not-empty) {
        let apply = if ($activation.stdout | str contains "def --env mise_hook [") {
          ["export def --env \"eon apply\" [] { mise_hook }", "mise eon apply"]
        } else {
          ["", ""]
        }
        $env.config.hooks.pre_prompt = (
          $env.config.hooks.pre_prompt? | default [] | prepend {
            condition: $eon_mise_inactive
            code: $"module mise {\n($activation.stdout)\n($apply.0)\n}\nuse mise\n($apply.1)"
          }
        )
      }
    }
  '';

  bashRc = pkgs.writeText "eon.bashrc" ''
    _eon_shell_init=''${BASH_SOURCE[0]%/*}/init
    # /etc/bashrc may replace the PATH that Runtime supplied before this file runs.
    _eon_bin=''${EON_SESSION_BIN:-''${BASH_SOURCE[0]%/*}/../bin}
    case ":$PATH:" in
      *":$_eon_bin:"*) ;;
      *) export PATH="$_eon_bin:$PATH" ;;
    esac
    _eon_default_ps1=$PS1
    _eon_default_ps2=$PS2
    if [[ -r ~/.bashrc ]]; then
      source ~/.bashrc
    fi
    if [[ $EON_SHELL_STARSHIP == 1 && $PS1 == "$_eon_default_ps1" \
        && $PS2 == "$_eon_default_ps2" && -z ''${PROMPT_COMMAND[*]-} ]] \
      && ! declare -F starship_precmd >/dev/null; then
      source "$_eon_shell_init/starship-bash" || printf '%s\n' 'eon: cannot initialize Starship' >&2
    fi
    if [[ $EON_SHELL_ZOXIDE == 1 ]] && ! declare -F __zoxide_hook >/dev/null; then
      source "$_eon_shell_init/zoxide-bash" || printf '%s\n' 'eon: cannot initialize Zoxide' >&2
    fi
    if [[ $EON_SHELL_ATUIN == 1 ]] && ! declare -F __atuin_history >/dev/null; then
      if [[ -n ''${ATUIN_NOBIND+x} ]]; then
        source "$_eon_shell_init/atuin-bash-nobind" || printf '%s\n' 'eon: cannot initialize Atuin' >&2
      else
        source "$_eon_shell_init/atuin-bash" || printf '%s\n' 'eon: cannot initialize Atuin' >&2
      fi
    fi
    if [[ $EON_SHELL_CARAPACE == 1 ]] && ! declare -F _carapace_completer >/dev/null; then
      _eon_completions=$(complete -p 2>/dev/null) || _eon_completions=
      source "$_eon_shell_init/carapace-bash" || printf '%s\n' 'eon: cannot initialize Carapace' >&2
      if [[ -n $_eon_completions ]]; then
        eval "$_eon_completions"
      fi
      unset _eon_completions
    fi
    # Last, so Eon's Starship still sees only the user's PROMPT_COMMAND.
    if [[ $EON_SHELL_MISE == 1 ]] && ! declare -F _mise_hook >/dev/null \
      && _eon_mise=$(type -P mise); then
      _eon_mise=$("$_eon_mise" activate bash 2>/dev/null) && [[ -n $_eon_mise ]] && eval "$_eon_mise"
    fi
    _eon_has_command_marks() {
      local hooks="''${PS0-} $PS1 ''${PROMPT_COMMAND[*]-} ''${preexec_functions[*]-} ''${precmd_functions[*]-}"
      local text=$hooks name IFS=$' \t\n'
      for name in ''${hooks//[^a-zA-Z0-9_]/ }; do
        text+=$(declare -f "$name")
      done
      [[ $text == *'133;C'* || $text == *'133;D'* ]]
    }
    if declare -F _eon_command_start >/dev/null || ! _eon_has_command_marks; then
      _eon_ps0="\''${ _eon_command_start \"\$_\"; }"
      _eon_command_mark() {
        if [[ -t 2 ]]; then
          printf '\e]133;%s\e\\' "$1" >&2
        else
          { printf '\e]133;%s\e\\' "$1" > /dev/tty; } 2>/dev/null || :
        fi
      }
      _eon_command_start() {
        local status=$? last_argument=$_
        if [[ -z ''${BLE_ATTACHED-} && " ''${PROMPT_COMMAND[*]-} " != *' _eon_command_end '* \
            && ''${PROMPT_COMMAND[*]-} != *'__bp_precmd_invoke_cmd'* ]]; then
          PROMPT_COMMAND=(_eon_command_end "''${PROMPT_COMMAND[@]}")
          # A user rc can replace the completion hook while a command runs.
          _eon_command_end "$status"
        fi
        if [[ ''${_eon_command_running-} != 1 ]]; then
          _eon_command_mark C
          _eon_command_running=1
        fi
        : "$last_argument"
      }
      _eon_command_end() {
        local status=''${1-$?}
        if [[ ''${_eon_command_running-} == 1 ]]; then
          _eon_command_mark "D;$status"
          unset _eon_command_running
        fi
        # Prompt tools may replace PS0 after startup.
        [[ ''${PS0-} == *"$_eon_ps0"* ]] || PS0+=$_eon_ps0
      }
      # Atuin's enter_accept invokes preexec_functions without expanding PS0.
      [[ " ''${preexec_functions[*]-} " == *' _eon_command_start '* ]] \
        || preexec_functions+=(_eon_command_start)
      if declare -F blehook >/dev/null; then
        [[ $(blehook PRECMD) == *'_eon_command_end'* ]] || blehook PRECMD+=_eon_command_end
      elif [[ ''${bash_preexec_imported-} || ''${__bp_imported-} ]]; then
        [[ " ''${precmd_functions[*]-} " == *' _eon_command_end '* ]] \
          || precmd_functions+=(_eon_command_end)
      elif [[ " ''${PROMPT_COMMAND[*]-} " != *' _eon_command_end '* ]]; then
        PROMPT_COMMAND=(_eon_command_end "''${PROMPT_COMMAND[@]}")
      fi
    fi
    unset -f _eon_has_command_marks
    unset _eon_bin _eon_shell_init _eon_default_ps1 _eon_default_ps2 _eon_mise
  '';

  shellTree = pkgs.runCommand "eon-shell" { } ''
    mkdir -p "$out"
    printf 'add_newline = false\n' > "$out/starship.toml"
    install -m644 ${bashRc} "$out/eon.bashrc"
    cp -r ${shellInit} "$out/init"
    for mask in $(seq 0 31); do
      mkdir -p "$out/nu/$mask"
      install -m644 ${nuBase} "$out/nu/$mask/eon.nu"
      if ((mask & 1)); then cat ${nuStarship} >> "$out/nu/$mask/eon.nu"; fi
      if ((mask & 2)); then cat ${nuZoxide} >> "$out/nu/$mask/eon.nu"; fi
      if ((mask & 4)); then
        if ((mask & 16)); then
          cat ${nuAtuinNoBind} >> "$out/nu/$mask/eon.nu"
        else
          cat ${nuAtuin} >> "$out/nu/$mask/eon.nu"
        fi
      fi
      if ((mask & 8)); then cat ${nuCarapace} >> "$out/nu/$mask/eon.nu"; fi
      cat ${nuMise} >> "$out/nu/$mask/eon.nu"
    done
  '';
in
shellTree
