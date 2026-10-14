{
  description = "Eon Linux alpha";

  nixConfig = {
    extra-substituters = [ "https://eon-yazelix.cachix.org" ];
    extra-trusted-public-keys = [
      "eon-yazelix.cachix.org-1:kPIp7u0Gjoz0GNzmRqeaqs798vU2raSOUpK+3xi7oK0="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/567a49d1913ce81ac6e9582e3553dd90a955875f";
    anima = {
      url = "github:Yazelix/anima/aa93bf16474f2e6692fdf96a82f6664fa03a18e6";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.kinestra.follows = "kinestra";
    };
    helix = {
      url = "github:Yazelix/nova-helix/817dcbca8b99c3db35ea66ded7492673d1a397ad";
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
    kinestra = {
      url = "github:Yazelix/kinestra/d1589705677dc001304584150557e67cef67f580";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      anima,
      helix,
      yazi,
      ratconfig,
      ghostty,
      kinestra,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      inherit (pkgs) lib;
      manifest = builtins.fromJSON (builtins.readFile ./components/eon-alpha-v3.json);
      eonCargo = builtins.fromTOML (builtins.readFile ./eon/crates/eon/Cargo.toml);
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
      # eon-manifest validates the graph. Each in-tree component's path must
      # hold the Cargo package Nix builds for it, at the declared version.
      workspacePackages = {
        orbit = "yazelix-orbit";
        venus = "yazelix-venus";
        eon-runtime = "eon-runtime";
        eon-workspace-protocol = "eon-workspace-protocol";
      };
      workspaceVersions = builtins.all (
        identity:
        identity.source.kind != "workspace"
        || (
          let
            cargo = (builtins.fromTOML (builtins.readFile ./${identity.source.path}/Cargo.toml)).package;
          in
          lib.assertMsg (
            cargo.name == workspacePackages.${identity.id} && cargo.version == identity.version
          ) "${identity.id} manifest path or version does not match its Cargo package"
        )
      ) manifest.components;
      # Cargo needs only the crates to build Orbit and Venus. Eon also compiles
      # in the graph and digests its Nix recipes into its generation identity.
      crateFiles = lib.fileset.difference (lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./orbit
        ./venus
        ./runtime
        ./eon
      ]) (lib.fileset.unions ([
        ./orbit/docs
        ./venus/docs
      ] ++ map (lib.fileset.fileFilter (file: file.hasExt "md")) [
        ./orbit
        ./venus
        ./runtime
        ./eon
      ]));
      crateSource = lib.fileset.toSource {
        root = ./.;
        fileset = crateFiles;
      };
      workspaceSource = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          crateFiles
          ./flake.nix
          ./flake.lock
          ./components
          ./nix/shell-environment.nix
        ];
      };
      workspaceCargoLock = {
        lockFile = ./Cargo.lock;
        outputHashes."dpi-0.1.1" = "sha256-fJQqcXhKPGtivKo1AyyGhy1oNebTFGyzWDmOesFsltI=";
        outputHashes."accesskit-0.24.1" = "sha256-W4Zk2BcKSPztJeeWBtQS3ugV0WPZWD0x8aX6R37zqj8=";
      };
      codexbarIdentity = component "codexbar";
      animaIdentity = component "anima";
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
      animaPackage = anima.packages.${system}.anima;

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

      codexbarPackage =
        assert codexbarIdentity.revision == "7c08c0ae783b72040b728757eb85a6cfe4fc2443";
        pkgs.stdenvNoCC.mkDerivation {
          pname = "codexbar";
          inherit (codexbarIdentity) version;
          src = pkgs.fetchurl {
            url = "https://github.com/steipete/CodexBar/releases/download/v${codexbarIdentity.version}/CodexBarCLI-v${codexbarIdentity.version}-linux-musl-x86_64.tar.gz";
            hash = "sha256-oC26jOydDYucyteY5LxuonAbfIAcud1z9vzf7b5QfcY=";
          };
          sourceRoot = ".";
          nativeBuildInputs = [ pkgs.makeWrapper ];
          dontBuild = true;
          dontStrip = true;
          installPhase = ''
            runHook preInstall
            test "$(cat VERSION)" = "$version"
            mkdir -p "$out/bin"
            cp CodexBarCLI VERSION "$out/bin/"
            cp -R CodexBar_CodexBarCore.bundle "$out/bin/"
            install -Dm444 ${./licenses/codexbar-MIT} "$out/share/licenses/codexbar/LICENSE"
            wrapProgram "$out/bin/CodexBarCLI" \
              --set-default SSL_CERT_FILE "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" \
              --set-default CURL_CA_BUNDLE "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
            "$out/bin/CodexBarCLI" --version | grep -F "$version"
            runHook postInstall
          '';
          meta = { license = lib.licenses.mit; platforms = [ system ]; };
        };

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
          pname = workspacePackages.orbit;
          inherit (orbitIdentity) version;
          src = crateSource;
          cargoLock = workspaceCargoLock;
          cargoBuildFlags = [
            "--package"
            workspacePackages.orbit
          ];
          cargoTestFlags = [
            "--package"
            workspacePackages.orbit
          ];
          # The accepted upstream suite needs host /bin/sh and set-ID chmod,
          # neither of which exists in the Nix build sandbox.
          doCheck = false;
          nativeBuildInputs = [
            pkgs.ncurses
            pkgs.zig_0_15.hook
          ];
          LIBGHOSTTY_VT_SYS_CPU = "baseline";
          GHOSTTY_SOURCE_DIR = ghostty;
          GHOSTTY_ZIG_SYSTEM_DIR = ghosttyDeps;
          dontUseZigBuild = true;
          dontUseZigCheck = true;
          dontUseZigInstall = true;
          postInstall = ''
            mkdir -p "$out/share/terminfo"
            ${pkgs.ncurses}/bin/tic -x -o "$out/share/terminfo" ${./orbit/terminfo/eon.terminfo}
            install -Dm444 ${./orbit/LICENSE} "$out/share/licenses/orbit/LICENSE"
            install -Dm444 ${ghostty}/LICENSE "$out/share/licenses/orbit/ghostty/LICENSE"
            install -Dm444 ${./licenses/libghostty-rs-MIT} "$out/share/licenses/orbit/libghostty-rs/LICENSE"
          '';
        };

      venusRuntimeLibraries = [
        pkgs.libxkbcommon
        pkgs.vulkan-loader
        pkgs.wayland
      ];
      venusPackage =
        pkgs.rustPlatform.buildRustPackage {
          pname = workspacePackages.venus;
          inherit (venusIdentity) version;
          src = crateSource;
          cargoLock = workspaceCargoLock;
          cargoBuildFlags = [
            "--package"
            workspacePackages.venus
          ];
          cargoTestFlags = [
            "--package"
            workspacePackages.venus
          ];
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
          FONTCONFIG_FILE = pkgs.makeFontsConf {
            fontDirectories = [
              pkgs.dejavu_fonts
              pkgs.nerd-fonts.symbols-only
              anima.packages.${system}.accelerando-font
            ];
          };
          postInstall = ''
            install -Dm444 ${./venus/LICENSE} "$out/share/licenses/venus/LICENSE"
            install -Dm444 ${./venus/THIRD_PARTY_NOTICES.md} "$out/share/licenses/venus/THIRD_PARTY_NOTICES.md"
          '';
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

      shellEnvironment = import ./nix/shell-environment.nix {
        inherit
          pkgs
          starshipPackage
          zoxidePackage
          atuinPackage
          carapacePackage
          zshPackage
          fishPackage
          ;
      };
      inherit (shellEnvironment)
        nuVendorAutoload
        bashRc
        zshConfig
        fishInit
        ;

      bashLicense = pkgs.runCommand "bash-license" { } ''
        ${pkgs.gnutar}/bin/tar -xOf ${bashPackage.src} bash-5.3/COPYING > "$out"
      '';
      zshLicense = pkgs.runCommand "zsh-license" { } ''
        ${pkgs.gnutar}/bin/tar -xOf ${zshPackage.src} zsh-5.9.1/LICENCE > "$out"
      '';

      desktopItem = pkgs.makeDesktopItem {
        name = "eon";
        desktopName = "Open Eon";
        comment = "Open an Eon workspace with persistent terminals";
        exec = "eon";
        actions.NewWindow = {
          name = "New Eon Window";
          exec = "eon window new";
        };
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
          "workspace"
        ];
      };

      directoryPickerConfig = pkgs.runCommand "eon-directory-picker-config" { } ''
        mkdir -p "$out"
        cat > "$out/keymap.toml" <<'EOF'
        [mgr]
        prepend_keymap = [
          { on = "<Enter>", run = "open --hovered", desc = "Use the highlighted folder for the Eon tab" },
          { on = "q", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "Q", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "<C-c>", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "<Esc>", run = "quit --no-cwd-file --code=130", desc = "Cancel folder selection" },
          { on = "<Tab>", run = "quit --code=10", desc = "Return to quick search" },
          { on = ["g", "/"], run = "cd /", desc = "Filesystem root" },
          # Keep file operations, openers, shells and suspension out of the picker.
          ${lib.concatMapStringsSep "\n" (key: "{ on = ${builtins.toJSON key}, run = \"noop\" },") [
            "o" "O" "<S-Enter>" "y" "x" "p" "P" "-" "_" "<C-->"
            "d" "D" "r" ";" ":" "<C-z>"
          ]}
        ]
        EOF
        cat > "$out/init.lua" <<'EOF'
        -- Replace file metadata with actions for this directory-only picker.
        for id = 1, 6 do
          Status:children_remove(id, id <= 3 and Status.LEFT or Status.RIGHT)
        end
        Status:children_add(function()
          return tostring(cx.layer) == "mgr"
            and " Enter Use folder · a Create · Tab Quick search · Shift+Z Jump · Esc/Ctrl+C Cancel · F1 Help"
            or ""
        end, 1000, Status.LEFT)
        EOF
      '';

      eonPackage =
        assert workspaceVersions;
        assert nixpkgs.rev == "567a49d1913ce81ac6e9582e3553dd90a955875f";
        assert ratconfig.rev == ratconfigIdentity.revision;
        assert manifest.product.target == system;
        assert anima.rev == animaIdentity.revision;
        assert lib.assertMsg ((builtins.fromTOML (builtins.readFile "${anima}/Cargo.toml")).package.version == animaIdentity.version)
          "Anima manifest version does not match the selected Cargo package";
        assert helix.rev == helixIdentity.revision;
        assert lib.assertMsg (helixCargo.workspace.package.version == helixIdentity.version)
          "Helix manifest version does not match the selected Cargo workspace package";
        pkgs.rustPlatform.buildRustPackage {
          pname = "eon";
          inherit (eonCargo.package) version;
          src = workspaceSource;
          cargoLock = workspaceCargoLock;
          cargoBuildFlags = [
            "--package"
            "eon"
          ];
          cargoTestFlags = [
            "--package"
            "eon"
            "--package"
            "eon-runtime"
          ];
          dontUseCargoParallelTests = true;
          preCheck = lib.optionalString pkgs.stdenv.isLinux ''
            # Keep ephemeral lifecycle state off the builder's disk journal.
            eonTestBuildTmpdir="$TMPDIR"
            export TMPDIR=/dev/shm
          '';
          postCheck = lib.optionalString pkgs.stdenv.isLinux ''
            export TMPDIR="$eonTestBuildTmpdir"
          '';
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
            for command in nu bash zsh fish lazygit; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/eon-$command"
            done
            ln -s ${starshipPackage}/bin/starship "$out/libexec/eon/bin/starship"
            ln -s ${zoxidePackage}/bin/zoxide "$out/libexec/eon/bin/zoxide"
            ln -s ${fzfPackage}/bin/fzf "$out/libexec/eon/bin/fzf"
            ln -s ${animaPackage}/bin/anima "$out/libexec/eon/bin/anima"
            ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-directory-picker"
            ln -s ${atuinPackage}/bin/atuin "$out/libexec/eon/bin/atuin"
            ln -s ${carapacePackage}/bin/carapace "$out/libexec/eon/bin/carapace"
            install -Dm444 ${./LICENSE} "$out/share/licenses/eon/LICENSE"
            install -Dm444 ${./runtime/LICENSE} "$out/share/licenses/eon/eon-runtime/LICENSE"
            install -Dm444 ${./runtime/NOTICE} "$out/share/licenses/eon/eon-runtime/NOTICE"
            install -Dm444 ${anima}/LICENSE "$out/share/licenses/eon/anima/LICENSE"
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
              --set EON_CODEXBAR "${codexbarPackage}/bin/CodexBarCLI" \
              --set EON_ANIMA "${animaPackage}/bin/anima" \
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
            description = "Native terminal workspaces with persistent terminals";
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
            install -Dm444 ${./runtime/LICENSE} "$out/share/licenses/eonterm/eon-runtime/LICENSE"
            install -Dm444 ${./runtime/NOTICE} "$out/share/licenses/eonterm/eon-runtime/NOTICE"
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
      coinstalledProducts = pkgs.buildEnv {
        name = "eon-products-profile-check";
        paths = [ eonPackage eontermPackage ];
      };
      eontermClosureCheck = pkgs.runCommand "eonterm-closure-check" { } ''
        test -x ${coinstalledProducts}/bin/eon
        test -x ${coinstalledProducts}/bin/eonterm
        for required in ${orbitPackage} ${venusPackage}; do
          ${pkgs.gnugrep}/bin/grep -Fx "$required" ${eontermClosure}/store-paths
        done
        for forbidden in \
          ${eonPackage} \
          ${codexbarPackage} \
          ${animaPackage} \
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
        eon-nu -c 'help clip copy | ignore; help clip paste | ignore'
        test "$(eon-nu --config ${nuVendorAutoload}/0/eon.nu -c 'scope aliases | where name == "clc" | get expansion.0')" = 'clip copy'
        test "$(eon-nu --config ${nuVendorAutoload}/0/eon.nu -c 'scope aliases | where name == "clp" | get expansion.0')" = 'clip paste'
        ${pkgs.coreutils}/bin/touch "$out"
      '';
      recordDemo = kinestra.lib.${system}.mkRecorder {
        name = "record-eon-demo";
        recipe = ./demo/record.rs;
        runtimeInputs = [ pkgs.swaybg ];
        environment = {
          EON_BIN = "${eonPackage}/bin/eon";
          ZOXIDE_BIN = "${zoxidePackage}/bin/zoxide";
          EON_DEMO_PATH = lib.makeBinPath [ pkgs.coreutils pkgs.bash pkgs.git ];
        };
      };
      br = pkgs.runCommand "br-0.7.2" { } ''
        mkdir -p "$out/bin" "$out/share/licenses/br"
        ${pkgs.gnutar}/bin/tar -xzf ${pkgs.fetchurl {
          url = "https://github.com/Dicklesworthstone/beads_rust/releases/download/v0.7.2/br-0.7.2-linux_musl_amd64.tar.gz";
          hash = "sha256-v6GvOaScpcGuj8ehPqOg05Y6qkuJEtcebDKVhHwJ12w=";
        }} -C "$out/bin" br LICENSE
        mv "$out/bin/LICENSE" "$out/share/licenses/br/LICENSE"
      '';
      bv = pkgs.runCommand "bv-0.25.2" { } ''
        mkdir -p "$out/bin" "$out/share/licenses/bv"
        ${pkgs.gnutar}/bin/tar -xzf ${pkgs.fetchurl {
          url = "https://github.com/Dicklesworthstone/beads_viewer/releases/download/v0.25.2/bv_0.25.2_linux_amd64.tar.gz";
          hash = "sha256-TCaQh696Dd72glYJy0zffbAWNiTSbXsCAww4wmCx4Zk=";
        }} -C "$out/bin" bv LICENSE
        mv "$out/bin/LICENSE" "$out/share/licenses/bv/LICENSE"
      '';
    in
    {
      packages.${system} = {
        default = eonPackage;
        eonterm = eontermPackage;
        installer = import ./nix/installer.nix { inherit pkgs; };
        manual-bundle = import ./nix/linux-bundle.nix {
          inherit pkgs eonPackage;
          manifestFile = ./components/eon-alpha-v3.json;
          lockFile = ./flake.lock;
        };
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
        record-demo = {
          type = "app";
          program = "${recordDemo}/bin/record-eon-demo";
        };
      };
      checks.${system} = {
        default = eonPackage;
        helix-grammars = helix.checks.${system}.codeberg_grammars;
        eonterm-closure = eontermClosureCheck;
        managed-command-path = managedCommandPathCheck;
        shell-environment = shellEnvironment.check;
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.cargo
          pkgs.rustc
          pkgs.clippy
          pkgs.rustfmt
          pkgs.git
          br
          bv
        ];
      };
    };
}
