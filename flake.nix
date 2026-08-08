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
      helixIdentity = component "helix";
      yaziIdentity = component "yazi";
      ratconfigIdentity = component "ratconfig";

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
            install -Dm444 ${./assets/eon.png} "$out/share/icons/hicolor/scalable/apps/eon.png"
          '';
          postFixup = ''
            wrapProgram "$out/bin/eon" \
              --set EON_ORBIT "${orbitPackage}/bin/yazelix-orbit" \
              --set EON_VENUS "${venusPackage}/bin/yazelix-venus" \
              --prefix PATH : "${
                lib.makeBinPath [
                  helixPackage
                  yaziPackage
                ]
              }" \
              --prefix TERMINFO_DIRS : "${orbitPackage}/share/terminfo"
          '';
          meta = {
            description = "Thin orchestrator for Eon for desktop and Sessions";
            license = lib.licenses.mit;
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
