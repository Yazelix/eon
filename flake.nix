{
  description = "Eon Linux alpha";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/567a49d1913ce81ac6e9582e3553dd90a955875f";
    orbit = {
      url = "git+https://github.com/Yazelix/eon-sessions.git?rev=00b136318bea13e3f08d490468f069de6f6b9bd2";
      flake = false;
    };
    venus = {
      url = "git+https://github.com/Yazelix/eon-desktop.git?rev=2d36c72dc87ca5416e22d6afdc35c6ab4e2fb832";
      flake = false;
    };
    protocol = {
      url = "git+https://github.com/Yazelix/eon-sessions.git?rev=9d6d2bb37f20ab4ad9e186c7bc715eabef43e757";
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
      protocol,
      helix,
      yazi,
      ratconfig,
      ghostty,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      inherit (pkgs) lib;
      manifest = builtins.fromJSON (builtins.readFile ./components/eon-alpha-v1.json);
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
      starshipIdentity = component "starship";
      zoxideIdentity = component "zoxide";
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
        assert orbit.rev == orbitIdentity.revision;
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
      protocolRevision = (builtins.head (builtins.head venusIdentity.requires).interfaces).proof;
      venusSource =
        assert protocol.rev == protocolRevision;
        pkgs.runCommand "eon-desktop-${venusIdentity.revision}" { } ''
          cp -R ${venus}/. "$out"
          chmod -R u+w "$out"
          ln -s ${protocol}/crates/protocol "$out/orbit-protocol"
          substituteInPlace "$out/Cargo.toml" \
            --replace-fail \
              'orbit-protocol = { git = "https://github.com/luccahuguet/orbit.git", rev = "${protocolRevision}" }' \
              'orbit-protocol = { path = "orbit-protocol" }'
          substituteInPlace "$out/Cargo.lock" \
            --replace-fail \
              'source = "git+https://github.com/luccahuguet/orbit.git?rev=${protocolRevision}#${protocolRevision}"' \
              ""
        '';
      venusPackage =
        assert venus.rev == venusIdentity.revision;
        pkgs.rustPlatform.buildRustPackage {
          pname = "yazelix-venus";
          inherit (venusIdentity) version;
          src = venusSource;
          cargoLock.lockFile = "${venusSource}/Cargo.lock";
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
          FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; };
          postFixup = ''
            wrapProgram "$out/bin/yazelix-venus" \
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
        assert helix.rev == helixIdentity.revision;
        helix.packages.${system}.yazelix_helix;

      nushellPackage = managedPackage nushellIdentity pkgs.nushell;
      starshipPackage = managedPackage starshipIdentity pkgs.starship;
      zoxidePackage = managedPackage zoxideIdentity pkgs.zoxide;
      lazygitPackage = managedPackage lazygitIdentity pkgs.lazygit;

      starshipInit = pkgs.runCommand "eon-starship.nu" { } ''
        ${starshipPackage}/bin/starship init nu > "$out"
      '';
      zoxideInit = pkgs.runCommand "eon-zoxide.nu" { } ''
        ${zoxidePackage}/bin/zoxide init nushell > "$out"
        substituteInPlace "$out" \
          --replace-fail '^zoxide' '^${zoxidePackage}/bin/zoxide'
      '';
      starshipConfig = pkgs.writeText "eon-starship.toml" (
        builtins.readFile ./defaults/starship.toml
      );
      nuEnv = pkgs.replaceVars ./defaults/nushell/env.nu {
        inherit starshipConfig;
      };
      nuConfig = pkgs.replaceVars ./defaults/nushell/config.nu {
        inherit starshipInit zoxideInit;
      };

      desktopItem = pkgs.makeDesktopItem {
        name = "eon";
        desktopName = "Eon";
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

      eonPackage =
        assert nixpkgs.rev == "567a49d1913ce81ac6e9582e3553dd90a955875f";
        assert ratconfig.rev == ratconfigIdentity.revision;
        assert manifest.product.target == system;
        pkgs.rustPlatform.buildRustPackage {
          pname = "eon";
          version = "0.1.0";
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./components/eon-alpha-v1.json
              ./crates
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--package"
            "eon"
          ];
          cargoTestFlags = [
            "--package"
            "eon"
          ];
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
            for command in nu hx yazi ya lazygit lg; do
              ln -s eon "$out/bin/eon-$command"
            done
            mkdir -p "$out/libexec/eon/bin"
            for command in nu hx yazi ya lazygit; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/$command"
            done
            install -Dm444 ${./LICENSE} "$out/share/licenses/eon/LICENSE"
            install -Dm444 ${nushellPackage.src}/LICENSE "$out/share/licenses/eon/nushell/LICENSE"
            install -Dm444 ${starshipPackage.src}/LICENSE "$out/share/licenses/eon/starship/LICENSE"
            install -Dm444 ${zoxidePackage.src}/LICENSE "$out/share/licenses/eon/zoxide/LICENSE"
            install -Dm444 ${helix}/LICENSE "$out/share/licenses/eon/helix/LICENSE"
            install -Dm444 ${yazi}/LICENSE "$out/share/licenses/eon/yazi/LICENSE"
            install -Dm444 ${lazygitPackage.src}/LICENSE "$out/share/licenses/eon/lazygit/LICENSE"
          '';
          postFixup = ''
            wrapProgram "$out/bin/eon" \
              --set EON_ORBIT "${orbitPackage}/bin/yazelix-orbit" \
              --set EON_VENUS "${venusPackage}/bin/yazelix-venus" \
              --set EON_SHELL "$out/bin/eon-nu" \
              --set EON_NU "${nushellPackage}/bin/nu" \
              --set EON_HX "${helixPackage}/bin/hx" \
              --set EON_YAZI "${yaziPackage}/bin/yazi" \
              --set EON_YA "${yaziPackage}/bin/ya" \
              --set EON_LAZYGIT "${lazygitPackage}/bin/lazygit" \
              --set EON_LG "$out/bin/eon-lazygit" \
              --set EON_NU_CONFIG "${nuConfig}" \
              --set EON_NU_ENV "${nuEnv}" \
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
    in
    {
      packages.${system}.default = eonPackage;
      apps.${system}.default = {
        type = "app";
        program = "${eonPackage}/bin/eon";
      };
      checks.${system}.default = eonPackage;
    };
}
