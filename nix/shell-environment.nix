# Startup files for the managed shells. Eon's Nix composition passes the
# exact pinned tools; the returned paths become opaque launch inputs.
{ pkgs, starshipPackage, zoxidePackage, atuinPackage, carapacePackage }:
let
  shellInit = pkgs.runCommand "eon-shell-init" { } ''
    mkdir -p "$out"
    ${starshipPackage}/bin/starship init nu > "$out/starship-nu"
    ${starshipPackage}/bin/starship init bash --print-full-init > "$out/starship-bash"

    for shell in nushell bash; do
      ${zoxidePackage}/bin/zoxide init "$shell" > "$out/zoxide-$shell"
    done
    substituteInPlace "$out/zoxide-nushell" \
      --replace-fail '^zoxide ' '^${zoxidePackage}/bin/zoxide '
    substituteInPlace "$out/zoxide-bash" \
      --replace-fail '\command zoxide ' '\command ${zoxidePackage}/bin/zoxide '

    mkdir -p "$TMPDIR/home" "$TMPDIR/config"
    for shell in nu bash; do
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

    for shell in nushell bash; do
      HOME="$TMPDIR/home" XDG_CONFIG_HOME=/eon-carapace-config \
        ${carapacePackage}/bin/carapace _carapace "$shell" \
        > "$out/carapace-$shell"
    done
    sed -i \
      '1c export PATH="''${XDG_CONFIG_HOME:-$HOME/.config}/carapace/bin:$PATH"' \
      "$out/carapace-bash"
    sed -i \
      's|xargs carapace |xargs ${carapacePackage}/bin/carapace |g' \
      "$out/carapace-bash"
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
in
{
  inherit nuVendorAutoload bashRc;
}
