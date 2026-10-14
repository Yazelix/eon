# Startup files for the managed shells. Eon's Nix composition passes the
# exact pinned tools; the returned paths become opaque launch inputs.
{ pkgs, starshipPackage, zoxidePackage, atuinPackage, carapacePackage, zshPackage, fishPackage }:
let
  shellInit = pkgs.runCommand "eon-shell-init" { } ''
    mkdir -p "$out"
    ${starshipPackage}/bin/starship init nu > "$out/starship-nu"
    for shell in bash zsh fish; do
      ${starshipPackage}/bin/starship init "$shell" --print-full-init \
        > "$out/starship-$shell"
    done

    for shell in nushell bash zsh fish; do
      ${zoxidePackage}/bin/zoxide init "$shell" > "$out/zoxide-$shell"
    done
    substituteInPlace "$out/zoxide-nushell" \
      --replace-fail '^zoxide ' '^${zoxidePackage}/bin/zoxide '
    for shell in bash zsh; do
      substituteInPlace "$out/zoxide-$shell" \
        --replace-fail '\command zoxide ' '\command ${zoxidePackage}/bin/zoxide '
    done
    substituteInPlace "$out/zoxide-fish" \
      --replace-fail 'command zoxide ' 'command ${zoxidePackage}/bin/zoxide '

    mkdir -p "$TMPDIR/home" "$TMPDIR/config"
    for shell in nu bash zsh fish; do
      HOME="$TMPDIR/home" XDG_CONFIG_HOME="$TMPDIR/config" \
        ${atuinPackage}/bin/atuin init "$shell" --disable-up-arrow --disable-ai \
        > "$out/atuin-$shell"
      ATUIN_NOBIND=1 HOME="$TMPDIR/home" XDG_CONFIG_HOME="$TMPDIR/config" \
        ${atuinPackage}/bin/atuin init "$shell" --disable-up-arrow --disable-ai \
        > "$out/atuin-$shell-nobind"
      sed -E -i \
        's#atuin (uuid|history|search)#${atuinPackage}/bin/atuin \1#g' \
        "$out/atuin-$shell" "$out/atuin-$shell-nobind"
    done

    for shell in nushell bash zsh fish; do
      HOME="$TMPDIR/home" XDG_CONFIG_HOME=/eon-carapace-config \
        ${carapacePackage}/bin/carapace _carapace "$shell" \
        > "$out/carapace-$shell"
    done
    sed -i \
      '1c export PATH="''${XDG_CONFIG_HOME:-$HOME/.config}/carapace/bin:$PATH"' \
      "$out/carapace-bash" "$out/carapace-zsh"
    sed -i \
      's|xargs carapace |xargs ${carapacePackage}/bin/carapace |g' \
      "$out/carapace-bash" "$out/carapace-zsh"
    sed -i \
      '1c set -l eon_carapace_config "$HOME/.config"; set -q XDG_CONFIG_HOME; and set eon_carapace_config "$XDG_CONFIG_HOME"; fish_add_path --path "$eon_carapace_config/carapace/bin"; set -e eon_carapace_config' \
      "$out/carapace-fish"
    sed -i \
      's|xargs carapace |xargs ${carapacePackage}/bin/carapace |g' \
      "$out/carapace-fish"
    tail -n +2 "$out/carapace-nushell" > "$TMPDIR/carapace-nushell"
    printf '%s\n' '$env.PATH = ($env.PATH | split row (char esep) | where { $in != (($env.XDG_CONFIG_HOME? | default ($env.HOME | path join ".config")) | path join "carapace" "bin") } | prepend (($env.XDG_CONFIG_HOME? | default ($env.HOME | path join ".config")) | path join "carapace" "bin"))' \
      > "$out/carapace-nushell"
    sed \
      's|  carapace $spans.0 nushell|  ^${carapacePackage}/bin/carapace $spans.0 nushell|' \
      "$TMPDIR/carapace-nushell" >> "$out/carapace-nushell"
  '';
  nuBase = pkgs.writeText "eon-nu-base" ''
    $env.config.show_banner = false
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
        overlay use ${shellInit}/starship-nu
      } catch {|error|
        print --stderr $"eon: cannot initialize Starship: ($error.msg)"
      }
    }
  '';
  nuZoxide = pkgs.writeText "eon-nu-zoxide" ''
    source ${shellInit}/zoxide-nushell
  '';
  nuAtuinSnippet =
    name: initializer:
    pkgs.writeText name ''
      if not (
        (scope commands | any {|command| $command.name == "_atuin_search_cmd" }) or
        ($env.config.keybindings? | default [] | any {|binding| ($binding.name? | default "") == "atuin" })
      ) {
        try {
          source ${shellInit}/${initializer}
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
        source ${shellInit}/carapace-nushell
      } catch {|error|
        print --stderr $"eon: cannot initialize Carapace: ($error.msg)"
      }
    }
  '';
  nuVendorAutoload = pkgs.runCommand "eon-nu-vendor-autoload" { } ''
    for mask in $(seq 0 31); do
      mkdir -p "$out/$mask"
      install -m644 ${nuBase} "$out/$mask/eon.nu"
      if ((mask & 1)); then cat ${nuStarship} >> "$out/$mask/eon.nu"; fi
      if ((mask & 2)); then cat ${nuZoxide} >> "$out/$mask/eon.nu"; fi
      if ((mask & 4)); then
        if ((mask & 16)); then
          cat ${nuAtuinNoBind} >> "$out/$mask/eon.nu"
        else
          cat ${nuAtuin} >> "$out/$mask/eon.nu"
        fi
      fi
      if ((mask & 8)); then cat ${nuCarapace} >> "$out/$mask/eon.nu"; fi
    done
  '';

  bashRc = pkgs.writeText "eon.bashrc" ''
    _eon_default_ps1=$PS1
    _eon_default_ps2=$PS2
    if [[ -r ~/.bashrc ]]; then
      source ~/.bashrc
    fi
    if [[ $EON_SHELL_STARSHIP == 1 && $PS1 == "$_eon_default_ps1" \
        && $PS2 == "$_eon_default_ps2" && -z ''${PROMPT_COMMAND[*]-} ]] \
      && ! declare -F starship_precmd >/dev/null; then
      source ${shellInit}/starship-bash || printf '%s\n' 'eon: cannot initialize Starship' >&2
    fi
    if [[ $EON_SHELL_ZOXIDE == 1 ]] && ! declare -F __zoxide_hook >/dev/null; then
      source ${shellInit}/zoxide-bash || printf '%s\n' 'eon: cannot initialize Zoxide' >&2
    fi
    if [[ $EON_SHELL_ATUIN == 1 ]] && ! declare -F __atuin_history >/dev/null; then
      if [[ -n ''${ATUIN_NOBIND+x} ]]; then
        source ${shellInit}/atuin-bash-nobind || printf '%s\n' 'eon: cannot initialize Atuin' >&2
      else
        source ${shellInit}/atuin-bash || printf '%s\n' 'eon: cannot initialize Atuin' >&2
      fi
    fi
    if [[ $EON_SHELL_CARAPACE == 1 ]] && ! declare -F _carapace_completer >/dev/null; then
      _eon_completions=$(complete -p 2>/dev/null) || _eon_completions=
      source ${shellInit}/carapace-bash || printf '%s\n' 'eon: cannot initialize Carapace' >&2
      if [[ -n $_eon_completions ]]; then
        eval "$_eon_completions"
      fi
      unset _eon_completions
    fi
    unset _eon_default_ps1 _eon_default_ps2
  '';

  zshEnv = pkgs.writeText "eon.zshenv" ''
    _eon_managed_zdotdir=$ZDOTDIR
    _eon_user_zdotdir=''${EON_USER_ZDOTDIR:-$HOME}
    if [[ $_eon_user_zdotdir != $_eon_managed_zdotdir ]]; then
      ZDOTDIR=$_eon_user_zdotdir
      [[ -r "$ZDOTDIR/.zshenv" ]] && source "$ZDOTDIR/.zshenv"
      _eon_user_zdotdir=''${ZDOTDIR:-$HOME}
    fi
    if [[ -o interactive ]]; then
      export EON_USER_ZDOTDIR=$_eon_user_zdotdir
      export ZDOTDIR=$_eon_managed_zdotdir
    else
      export ZDOTDIR=$_eon_user_zdotdir
      unset EON_USER_ZDOTDIR
    fi
    unset _eon_managed_zdotdir _eon_user_zdotdir
  '';
  zshRc = pkgs.writeText "eon.zshrc" ''
    _eon_default_prompt=$PROMPT
    _eon_default_rprompt=$RPROMPT
    ZDOTDIR=$EON_USER_ZDOTDIR
    [[ -r "$ZDOTDIR/.zshrc" ]] && source "$ZDOTDIR/.zshrc"
    if [[ $EON_SHELL_STARSHIP == 1 && $PROMPT == $_eon_default_prompt \
      && $RPROMPT == $_eon_default_rprompt ]] \
      && (( ! $+functions[prompt_starship_precmd] )); then
      source ${shellInit}/starship-zsh || print -u2 'eon: cannot initialize Starship'
    fi
    if [[ $EON_SHELL_ZOXIDE == 1 ]] && (( ! $+functions[__zoxide_hook] )); then
      source ${shellInit}/zoxide-zsh
    fi
    if [[ $EON_SHELL_ATUIN == 1 ]] && (( ! $+functions[_atuin_search] )); then
      if [[ -n ''${ATUIN_NOBIND+x} ]]; then
        source ${shellInit}/atuin-zsh-nobind || print -u2 'eon: cannot initialize Atuin'
      else
        source ${shellInit}/atuin-zsh || print -u2 'eon: cannot initialize Atuin'
      fi
    fi
    if [[ $EON_SHELL_CARAPACE == 1 ]] && (( ! $+functions[_carapace_completer] )); then
      if (( $+functions[compdef] )); then
        typeset -A _eon_completions
        _eon_completions=("''${(@kv)_comps}")
      else
        autoload -Uz compinit && compinit -D
      fi
      source ${shellInit}/carapace-zsh || print -u2 'eon: cannot initialize Carapace'
      if (( $+parameters[_eon_completions] )); then
        _comps+=("''${(@kv)_eon_completions}")
        unset _eon_completions
      fi
    fi
    unset _eon_default_prompt _eon_default_rprompt EON_USER_ZDOTDIR
  '';
  zshConfig = pkgs.linkFarm "eon-zsh-config" [
    {
      name = ".zshenv";
      path = zshEnv;
    }
    {
      name = ".zshrc";
      path = zshRc;
    }
  ];
  shellEnvironmentCheck = pkgs.runCommand "eon-shell-environment-check" { } ''
    mkdir -p "$TMPDIR/home" "$TMPDIR/zdot" "$TMPDIR/fish/fish"
    HOME="$TMPDIR/home" ZDOTDIR=${zshConfig} EON_USER_ZDOTDIR="$TMPDIR/zdot" \
      EON_SHELL_STARSHIP=0 EON_SHELL_ZOXIDE=0 EON_SHELL_ATUIN=0 \
      EON_SHELL_CARAPACE=1 ${zshPackage}/bin/zsh -i -c \
        '(( $+functions[_carapace_completer] ))'
    test ! -e "$TMPDIR/zdot/.zcompdump"

    printf '%s\n' 'complete -c git -a eon_user_marker' \
      'complete -c eon-native -a eon_native_marker' 'complete -c acpi -e' \
      > "$TMPDIR/fish/fish/config.fish"
    export HOME="$TMPDIR/home" XDG_CONFIG_HOME="$TMPDIR/fish"
    export EON_SHELL_STARSHIP=0 EON_SHELL_ZOXIDE=0 EON_SHELL_ATUIN=0
    EON_SHELL_CARAPACE=1 ${fishPackage}/bin/fish -i -C 'source ${fishInit}' -c '
        complete -c git | string match -q "*eon_user_marker*"; or exit 1
        complete -c acpi | string match -q "*_carapace_completer*"; or exit 1
        set before (complete -c git | count)
        set native_before (complete -c eon-native | count)
        test $native_before -eq 1; or exit 1
        source ${fishInit}
        test $before -eq (complete -c git | count); or exit 1
        test $native_before -eq (complete -c eon-native | count)
      '
    EON_SHELL_CARAPACE=0 ${fishPackage}/bin/fish -i -C 'source ${fishInit}' -c '
        not functions -q _carapace_completer; or exit 1
        complete -c git | string match -q "*eon_user_marker*"; or exit 1
      '
    touch "$out"
  '';

  fishInit = pkgs.writeText "eon.fish" ''
    status is-interactive; or return
    if test "$EON_SHELL_STARSHIP" = 1 \
      && test (functions --details fish_prompt) = embedded:functions/fish_prompt.fish \
      && not functions -q fish_right_prompt \
      && not functions -q __starship_set_job_count
      source ${shellInit}/starship-fish; or echo 'eon: cannot initialize Starship' >&2
    end
    if test "$EON_SHELL_ZOXIDE" = 1; and not functions -q __zoxide_hook
      source ${shellInit}/zoxide-fish; or echo 'eon: cannot initialize Zoxide' >&2
    end
    if test "$EON_SHELL_ATUIN" = 1; and not functions -q _atuin_search
      if set -q ATUIN_NOBIND
        source ${shellInit}/atuin-fish-nobind; or echo 'eon: cannot initialize Atuin' >&2
      else
        source ${shellInit}/atuin-fish; or echo 'eon: cannot initialize Atuin' >&2
      end
    end
    if test "$EON_SHELL_CARAPACE" = 1; and not functions -q _carapace_completer
      set -l _eon_completions (complete)
      source ${shellInit}/carapace-fish; or echo 'eon: cannot initialize Carapace' >&2
      set -l _eon_active_completions (complete)
      # ponytail: startup-only snapshot scan; index only if user completion sets make it measurable.
      for _eon_completion in $_eon_completions
        contains -- $_eon_completion $_eon_active_completions; or eval $_eon_completion
      end
    end
  '';
in
{
  inherit nuVendorAutoload bashRc zshConfig fishInit;
  check = shellEnvironmentCheck;
}
