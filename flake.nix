{
  description = "Eon Linux alpha";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/567a49d1913ce81ac6e9582e3553dd90a955875f";
    orbit = {
      url = "git+https://github.com/Yazelix/eon-sessions.git?rev=91999d79546422b49bdbc124166a65859d0bd872";
      flake = false;
    };
    venus = {
      url = "git+https://github.com/Yazelix/eon-desktop.git?rev=94b15af20d1798b648f4d9945fd6bb647f10add8";
      flake = false;
    };
    helix = {
      url = "github:luccahuguet/yazelix-helix/7e6cd307d00783c16ad4cff99ed71936d34f6572";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    yazi = {
      url = "github:sxyazi/yazi/aa526434f00bb44e2e902d9a4ac5f810da1018b9";
      flake = false;
    };
    ratconfig = {
      url = "github:luccahuguet/ratconfig/e6ec2ebfe84b2358186410680cbcaf0564eb59a2";
      flake = false;
    };
    ghostty = {
      url = "github:ghostty-org/ghostty/a887df42c56f6de86c0fe6da9c4eeca37931e083";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      orbit,
      venus,
      helix,
      yazi,
      ratconfig,
      ghostty,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      inherit (pkgs) lib;
      manifest = builtins.fromJSON (builtins.readFile ./components/eon-alpha-v3.json);
      eonCargo = builtins.fromTOML (builtins.readFile ./crates/eon/Cargo.toml);
      orbitCargo = builtins.fromTOML (builtins.readFile "${orbit}/Cargo.toml");
      venusCargo = builtins.fromTOML (builtins.readFile "${venus}/Cargo.toml");
      helixCargo = builtins.fromTOML (builtins.readFile "${helix}/Cargo.toml");
      component =
        id:
        let
          matches = builtins.filter (candidate: candidate.id == id) manifest.components;
        in
        if builtins.length matches == 1 then
          builtins.head matches
        else
          throw "Eon manifest must contain exactly one ${id} component";
      orbitIdentity = component "orbit";
      venusIdentity = component "venus";
      nushellIdentity = component "nushell";
      bashIdentity = component "bash";
      zshIdentity = component "zsh";
      fishIdentity = component "fish";
      starshipIdentity = component "starship";
      zoxideIdentity = component "zoxide";
      fzfIdentity = component "fzf";
      atuinIdentity = component "atuin";
      carapaceIdentity = component "carapace";
      helixIdentity = component "helix";
      yaziIdentity = component "yazi";
      lazygitIdentity = component "lazygit";
      ratconfigIdentity = component "ratconfig";

      githubSource =
        identity: hash:
        let
          match = builtins.match "https://github.com/([^/]+)/([^/]+)\\.git" identity.source.url;
        in
        assert identity.source.kind == "git";
        assert match != null;
        pkgs.fetchFromGitHub {
          owner = builtins.elemAt match 0;
          repo = builtins.elemAt match 1;
          rev = identity.revision;
          inherit hash;
        };
      managedPackage =
        identity: package:
        assert package.version == identity.version;
        package.overrideAttrs (_: {
          src = githubSource identity package.src.outputHash;
        });

      ghosttyDeps = pkgs.zig_0_15.fetchDeps {
        pname = "ghostty";
        version = "0-unstable";
        src = ghostty;
        fetchAll = true;
        hash = "sha256-PnM+hZIlLyQwK8vJgd/Bhjt1lNIz06T8FahwliRmMrY=";
      };

      orbitPackage =
        assert ghostty.rev == "a887df42c56f6de86c0fe6da9c4eeca37931e083";
        pkgs.rustPlatform.buildRustPackage {
          pname = "yazelix-orbit";
          inherit (orbitIdentity) version;
          src = orbit;
          cargoLock.lockFile = "${orbit}/Cargo.lock";
          cargoBuildFlags = [
            "--package"
            "yazelix-orbit"
          ];
          cargoTestFlags = [
            "--package"
            "yazelix-orbit"
          ];
          # The accepted upstream suite needs host /bin/sh and set-ID chmod,
          # neither of which exists in the Nix build sandbox.
          doCheck = false;
          nativeBuildInputs = [
            pkgs.ncurses
            pkgs.zig_0_15.hook
          ];
          GHOSTTY_SOURCE_DIR = ghostty;
          GHOSTTY_ZIG_SYSTEM_DIR = ghosttyDeps;
          dontUseZigBuild = true;
          dontUseZigCheck = true;
          dontUseZigInstall = true;
          postInstall = ''
            mkdir -p "$out/share/terminfo"
            ${pkgs.ncurses}/bin/tic -x -o "$out/share/terminfo" ${orbit}/terminfo/eon.terminfo
          '';
        };

      venusRuntimeLibraries = [
        pkgs.libxkbcommon
        pkgs.vulkan-loader
        pkgs.wayland
      ];
      venusProtocolSourceRevision = "91999d79546422b49bdbc124166a65859d0bd872";
      workspaceProtocolRevision = "c305453bba4fe50c29f65e829b9cd65af31ced8a";
      venusSource =
        assert orbit.rev == (builtins.head venusIdentity.requires).revision;
        pkgs.runCommand "eon-desktop-${venusIdentity.revision}" { } ''
          cp -R ${venus}/. "$out"
          chmod -R u+w "$out"
          ln -s ${orbit}/crates/protocol "$out/orbit-protocol"
          ln -s ${./crates/eon-workspace-protocol} "$out/eon-workspace-protocol"
          substituteInPlace "$out/Cargo.toml" \
            --replace-fail \
              'orbit-protocol = { git = "https://github.com/Yazelix/eon-sessions.git", rev = "${venusProtocolSourceRevision}" }' \
              'orbit-protocol = { path = "orbit-protocol" }'
          substituteInPlace "$out/Cargo.toml" \
            --replace-fail \
              'eon-workspace-protocol = { git = "https://github.com/Yazelix/eon.git", rev = "${workspaceProtocolRevision}" }' \
              'eon-workspace-protocol = { path = "eon-workspace-protocol" }'
          substituteInPlace "$out/Cargo.lock" \
            --replace-fail \
              'source = "git+https://github.com/Yazelix/eon-sessions.git?rev=${venusProtocolSourceRevision}#${venusProtocolSourceRevision}"' \
              ""
          substituteInPlace "$out/Cargo.lock" \
            --replace-fail \
              'source = "git+https://github.com/Yazelix/eon.git?rev=${workspaceProtocolRevision}#${workspaceProtocolRevision}"' \
              ""
        '';
      venusPackage =
        pkgs.rustPlatform.buildRustPackage {
          pname = "yazelix-venus";
          inherit (venusIdentity) version;
          src = venusSource;
          cargoLock = {
            lockFile = "${venusSource}/Cargo.lock";
            outputHashes."dpi-0.1.1" = "sha256-ahkXE2sS1RGZDpaZEKCRMrwctWKWEnOHyn+JsUNs9Ws=";
          };
          nativeBuildInputs = [
            pkgs.makeWrapper
            pkgs.pkg-config
          ];
          buildInputs = [
            pkgs.fontconfig
            pkgs.freetype
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxi
            pkgs.libxrandr
            pkgs.libxkbcommon
            pkgs.wayland
          ];
          FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts pkgs.nerd-fonts.symbols-only ]; };
          postFixup = ''
            wrapProgram "$out/bin/yazelix-venus" \
              --set FONTCONFIG_FILE "$FONTCONFIG_FILE" \
              --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath venusRuntimeLibraries}" \
              --prefix XDG_DATA_DIRS : "${pkgs.mesa}/share"
          '';
        };

      yaziPackage =
        assert yazi.rev == yaziIdentity.revision;
        assert pkgs.yazi-unwrapped.version == yaziIdentity.version;
        pkgs.yazi-unwrapped.overrideAttrs (old: {
          passthru = old.passthru // {
            srcs = old.passthru.srcs // {
              code_src = yazi // {
                name = "source";
              };
            };
          };
        });

      helixPackage =
        helix.packages.${system}.yazelix_helix;

      nushellPackage = managedPackage nushellIdentity pkgs.nushell;
      bashPackage =
        assert bashIdentity.revision == "b8c60bc9ca365f8261fa97900b6fa939f6ebc303";
        assert pkgs.bashInteractive.version == bashIdentity.version;
        pkgs.bashInteractive;
      zshPackage =
        assert zshIdentity.revision == "0e0d4ea11731c47f57bad042fbe75e3979d8a1d2";
        assert pkgs.zsh.version == zshIdentity.version;
        pkgs.zsh;
      fishPackage = managedPackage fishIdentity pkgs.fish;
      starshipPackage = managedPackage starshipIdentity pkgs.starship;
      zoxidePackage = managedPackage zoxideIdentity pkgs.zoxide;
      fzfPackage = managedPackage fzfIdentity pkgs.fzf;
      atuinPackage = managedPackage atuinIdentity pkgs.atuin;
      carapacePackage = managedPackage carapaceIdentity pkgs.carapace;
      lazygitPackage = managedPackage lazygitIdentity pkgs.lazygit;

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

      bashLicense = pkgs.runCommand "bash-license" { } ''
        ${pkgs.gnutar}/bin/tar -xOf ${bashPackage.src} bash-5.3/COPYING > "$out"
      '';
      zshLicense = pkgs.runCommand "zsh-license" { } ''
        ${pkgs.gnutar}/bin/tar -xOf ${zshPackage.src} zsh-5.9.1/LICENCE > "$out"
      '';

      desktopItem = pkgs.makeDesktopItem {
        name = "eon";
        desktopName = "Open Eon";
        comment = "Launch Eon for desktop and Sessions";
        exec = "eon";
        icon = "eon";
        startupWMClass = "yazelix-venus";
        terminal = false;
        categories = [
          "System"
          "TerminalEmulator"
        ];
        keywords = [
          "terminal"
          "shell"
          "sessions"
        ];
      };

      directoryPickerConfig = pkgs.runCommand "eon-directory-picker-config" { } ''
        mkdir -p "$out"
        cat > "$out/keymap.toml" <<'EOF'
        [mgr]
        keymap = [
          { on = "<Enter>", run = "quit", desc = "Use this folder for the Eon tab" },
          { on = "q", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "Q", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "<C-c>", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "<Esc>", run = "escape", desc = "Clear search or filter" },
          { on = "<Up>", run = "arrow prev", desc = "Previous entry" },
          { on = "k", run = "arrow prev", desc = "Previous entry" },
          { on = "<Down>", run = "arrow next", desc = "Next entry" },
          { on = "j", run = "arrow next", desc = "Next entry" },
          { on = "<Left>", run = "leave", desc = "Parent folder" },
          { on = "h", run = "leave", desc = "Parent folder" },
          { on = "<Right>", run = "enter", desc = "Enter folder" },
          { on = "l", run = "enter", desc = "Enter folder" },
          { on = "<PageUp>", run = "arrow -100%", desc = "Previous page" },
          { on = "<PageDown>", run = "arrow 100%", desc = "Next page" },
          { on = "Z", run = "plugin zoxide", desc = "Jump to a known folder without selecting it" },
          { on = ["g", "h"], run = "cd ~", desc = "Home folder" },
          { on = ["g", "/"], run = "cd /", desc = "Filesystem root" },
          { on = ["g", "<Space>"], run = "cd --interactive", desc = "Go to a folder path" },
          { on = ".", run = "hidden toggle", desc = "Show or hide hidden entries" },
          { on = "f", run = "filter --smart", desc = "Filter entries in this folder" },
          { on = "<F1>", run = "help", desc = "Folder picker keys" },
        ]
        EOF
        cat > "$out/yazi.toml" <<'EOF'
        [plugin]
        fetchers = []
        preloaders = []
        previewers = [{ url = "*/", run = "folder" }]
        EOF
        cat > "$out/init.lua" <<'EOF'
        -- Replace file metadata with actions for this directory-only picker.
        for id = 1, 6 do
          Status:children_remove(id, id <= 3 and Status.LEFT or Status.RIGHT)
        end
        Status:children_add(function()
          return tostring(cx.layer) == "mgr"
            and " Enter Use current folder · Shift+Z Jump · Ctrl+C Cancel · F1 Help"
            or ""
        end, 1000, Status.LEFT)
        EOF
      '';

      eonSource =
        let
          source = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./flake.nix
              ./flake.lock
              ./components/eon-alpha-v3.json
              ./crates
            ];
          };
        in
        pkgs.runCommand "eon-${self.rev or self.dirtyRev or "source"}" { } ''
          cp -R ${source}/. "$out"
          chmod -R u+w "$out"
          ln -s ${orbit}/crates/protocol "$out/crates/orbit-protocol"
          substituteInPlace "$out/crates/eon/Cargo.toml" \
            --replace-fail \
              'orbit-protocol = { git = "https://github.com/Yazelix/eon-sessions.git", rev = "${orbitIdentity.revision}" }' \
              'orbit-protocol = { path = "../orbit-protocol" }'
          substituteInPlace "$out/Cargo.lock" \
            --replace-fail \
              'source = "git+https://github.com/Yazelix/eon-sessions.git?rev=${orbitIdentity.revision}#${orbitIdentity.revision}"' \
              ""
        '';

      eonPackage =
        assert nixpkgs.rev == "567a49d1913ce81ac6e9582e3553dd90a955875f";
        assert ratconfig.rev == ratconfigIdentity.revision;
        assert manifest.product.target == system;
        assert orbit.rev == orbitIdentity.revision;
        assert lib.assertMsg (orbitCargo.package.version == orbitIdentity.version)
          "Orbit manifest version does not match the selected Cargo package";
        assert venus.rev == venusIdentity.revision;
        assert lib.assertMsg (venusCargo.package.version == venusIdentity.version)
          "Venus manifest version does not match the selected Cargo package";
        assert helix.rev == helixIdentity.revision;
        assert lib.assertMsg (helixCargo.workspace.package.version == helixIdentity.version)
          "Helix manifest version does not match the selected Cargo workspace package";
        pkgs.rustPlatform.buildRustPackage {
          pname = "eon";
          inherit (eonCargo.package) version;
          src = eonSource;
          cargoLock.lockFile = "${eonSource}/Cargo.lock";
          cargoBuildFlags = [
            "--package"
            "eon"
          ];
          cargoTestFlags = [
            "--package"
            "eon"
          ];
          dontUseCargoParallelTests = true;
          nativeBuildInputs = [
            pkgs.copyDesktopItems
            pkgs.makeWrapper
          ];
          desktopItems = [ desktopItem ];
          postInstall = ''
            for size in 16 24 32 48 64 128 256 512; do
              icon_dir="$out/share/icons/hicolor/''${size}x''${size}/apps"
              mkdir -p "$icon_dir"
              ${pkgs.imagemagick}/bin/magick ${./assets/eon.png} \
                -filter Box -resize "''${size}x''${size}" "$icon_dir/eon.png"
            done
            for command in nu bash zsh fish hx yazi ya lazygit lg; do
              ln -s eon "$out/bin/eon-$command"
            done
            mkdir -p "$out/libexec/eon/bin"
            for command in nu bash zsh fish hx yazi ya lazygit; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/$command"
            done
            for command in nu bash zsh fish; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/eon-$command"
            done
            ln -s ${starshipPackage}/bin/starship "$out/libexec/eon/bin/starship"
            ln -s ${zoxidePackage}/bin/zoxide "$out/libexec/eon/bin/zoxide"
            ln -s ${fzfPackage}/bin/fzf "$out/libexec/eon/bin/fzf"
            ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-directory-picker"
            ln -s ${atuinPackage}/bin/atuin "$out/libexec/eon/bin/atuin"
            ln -s ${carapacePackage}/bin/carapace "$out/libexec/eon/bin/carapace"
            install -Dm444 ${./LICENSE} "$out/share/licenses/eon/LICENSE"
            install -Dm444 ${nushellPackage.src}/LICENSE "$out/share/licenses/eon/nushell/LICENSE"
            install -Dm444 ${bashLicense} "$out/share/licenses/eon/bash/COPYING"
            install -Dm444 ${zshLicense} "$out/share/licenses/eon/zsh/LICENCE"
            install -Dm444 ${fishPackage.src}/COPYING "$out/share/licenses/eon/fish/COPYING"
            install -Dm444 ${starshipPackage.src}/LICENSE "$out/share/licenses/eon/starship/LICENSE"
            install -Dm444 ${zoxidePackage.src}/LICENSE "$out/share/licenses/eon/zoxide/LICENSE"
            install -Dm444 ${fzfPackage.src}/LICENSE "$out/share/licenses/eon/fzf/LICENSE"
            install -Dm444 ${atuinPackage.src}/LICENSE "$out/share/licenses/eon/atuin/LICENSE"
            install -Dm444 ${carapacePackage.src}/LICENSE "$out/share/licenses/eon/carapace/LICENSE"
            install -Dm444 ${helix}/LICENSE "$out/share/licenses/eon/helix/LICENSE"
            install -Dm444 ${yazi}/LICENSE "$out/share/licenses/eon/yazi/LICENSE"
            install -Dm444 ${lazygitPackage.src}/LICENSE "$out/share/licenses/eon/lazygit/LICENSE"
          '';
          postFixup = ''
            substituteInPlace "$out/share/applications/eon.desktop" \
              --replace-fail 'Exec=eon' "Exec=$out/bin/eon"
            wrapProgram "$out/bin/eon" \
              --set EON_ORBIT "${orbitPackage}/bin/yazelix-orbit" \
              --set EON_VENUS "${venusPackage}/bin/yazelix-venus" \
              --set EON_NU "${nushellPackage}/bin/nu" \
              --set EON_BASH "${bashPackage}/bin/bash" \
              --set EON_ZSH "${zshPackage}/bin/zsh" \
              --set EON_FISH "${fishPackage}/bin/fish" \
              --set EON_HX "${helixPackage}/bin/hx" \
              --set EON_YAZI "${yaziPackage}/bin/yazi" \
              --set EON_DIRECTORY_PICKER_CONFIG "${directoryPickerConfig}" \
              --set EON_YA "${yaziPackage}/bin/ya" \
              --set EON_LAZYGIT "${lazygitPackage}/bin/lazygit" \
              --set EON_NU_VENDOR_AUTOLOAD "${nuVendorAutoload}" \
              --set EON_BASH_RC "${bashRc}" \
              --set EON_ZSH_CONFIG "${zshConfig}" \
              --set EON_FISH_INIT "${fishInit}" \
              --set EON_SESSION_BIN "$out/libexec/eon/bin" \
              --prefix TERMINFO_DIRS : "${orbitPackage}/share/terminfo"
          '';
          meta = {
            description = "Thin orchestrator for Eon for desktop and Sessions";
            license = lib.licenses.asl20;
            mainProgram = "eon";
            platforms = [ system ];
          };
        };
      eontermPackage =
        assert manifest.composition.services == [ orbitIdentity.id ];
        assert manifest.composition.clients == [ venusIdentity.id ];
        eonPackage.overrideAttrs (_: {
          pname = "eonterm";
          desktopItems = [ ];
          postInstall = ''
            mv "$out/bin/eon" "$out/bin/eonterm"
            install -Dm444 ${./LICENSE} "$out/share/licenses/eonterm/LICENSE"
          '';
          postFixup = ''
            wrapProgram "$out/bin/eonterm" \
              --set EON_ORBIT "${orbitPackage}/bin/yazelix-orbit" \
              --set EON_VENUS "${venusPackage}/bin/yazelix-venus" \
              --unset EON_SESSION_BIN \
              --prefix TERMINFO_DIRS : "${orbitPackage}/share/terminfo"
          '';
          meta = {
            description = "Reusable Eon terminal product";
            license = lib.licenses.asl20;
            mainProgram = "eonterm";
            platforms = [ system ];
          };
        });
      eontermClosure = pkgs.closureInfo { rootPaths = [ eontermPackage ]; };
      eontermClosureCheck = pkgs.runCommand "eonterm-closure-check" { } ''
        for required in ${orbitPackage} ${venusPackage}; do
          ${pkgs.gnugrep}/bin/grep -Fx "$required" ${eontermClosure}/store-paths
        done
        for forbidden in \
          ${eonPackage} \
          ${nushellPackage} ${bashPackage} ${zshPackage} ${fishPackage} \
          ${starshipPackage} ${zoxidePackage} ${atuinPackage} ${carapacePackage} \
          ${fzfPackage} ${helixPackage} ${yaziPackage} ${lazygitPackage}; do
          ! ${pkgs.gnugrep}/bin/grep -Fx "$forbidden" ${eontermClosure}/store-paths
        done
        test ! -e ${eontermPackage}/libexec/eon
        ${pkgs.gnugrep}/bin/grep -F 'unset EON_SESSION_BIN' ${eontermPackage}/bin/eonterm
        ! ${pkgs.gnugrep}/bin/grep -F '/libexec/eon/bin' ${eontermPackage}/bin/eonterm
        ${pkgs.coreutils}/bin/touch "$out"
      '';
      managedCommandPathCheck = pkgs.runCommand "eon-managed-command-path-check" { } ''
        export HOME="$TMPDIR" EON_CONFIG_HOME="$TMPDIR/config" PATH=${eonPackage}/libexec/eon/bin
        for command in nu bash zsh fish; do
          version_output=$("$command" --version)
          test -n "$version_output" && test "$("eon-$command" --version)" = "$version_output"
        done
        ${pkgs.coreutils}/bin/touch "$out"
      '';
    in
    {
      packages.${system} = {
        default = eonPackage;
        eonterm = eontermPackage;
      };
      apps.${system} = {
        default = {
          type = "app";
          program = "${eonPackage}/bin/eon";
        };
        eonterm = {
          type = "app";
          program = "${eontermPackage}/bin/eonterm";
        };
      };
      checks.${system} = {
        default = eonPackage;
        eonterm-closure = eontermClosureCheck;
        managed-command-path = managedCommandPathCheck;
        shell-environment = shellEnvironmentCheck;
      };
    };
}
