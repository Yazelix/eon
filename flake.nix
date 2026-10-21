{
  description = "Eon";

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
    # Orbit's build-time Bend compiler (orbit/build.rs).
    bend = {
      url = "github:bendlang/bend/234cb9b950800f20b15feafcb4ce2f43e76e7fae";
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
      bend,
      kinestra,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      inherit (pkgs) lib;
      manifest = builtins.fromJSON (builtins.readFile ./components/eon-v4.json);
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
        ./orbit/LAWS.bend
        ./orbit/PROOF.bend
        ./orbit/proof
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
      # Each wrapper environment has one definition. makeWrapper renders it here, and
      # nix/bundle-relink.nix renders the same attrset into the bundle launcher's table.
      wrapperArgs =
        {
          set ? { },
          setDefault ? { },
          prefix ? { },
        }:
        lib.escapeShellArgs (
          lib.concatLists (
            lib.mapAttrsToList (name: value: [ "--set" name value ]) set
            ++ lib.mapAttrsToList (name: value: [ "--set-default" name value ]) setDefault
            ++ lib.mapAttrsToList (name: paths: [ "--prefix" name ":" (lib.concatStringsSep ":" paths) ]) prefix
          )
        );
      managedPackage =
        identity: package:
        assert package.version == identity.version;
        package.overrideAttrs (_: {
          src = githubSource identity package.src.outputHash;
        });

      codexbarEnvironment.setDefault = {
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
        CURL_CA_BUNDLE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      };
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
            wrapProgram "$out/bin/CodexBarCLI" ${wrapperArgs codexbarEnvironment}
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
      # libghostty's build script compiles the pinned Ghostty source with Zig
      # instead of cloning it; Orbit's package and the dev shell share this.
      ghosttyBuildEnv = {
        LIBGHOSTTY_VT_SYS_CPU = "baseline";
        GHOSTTY_SOURCE_DIR = ghostty;
        GHOSTTY_ZIG_SYSTEM_DIR = ghosttyDeps;
      };

      # orbit/build.rs runs the pinned Bend compiler and compiles its C with this
      # clang, named rather than put on PATH so it never becomes the dev shell's
      # cc. Without BEND_NO_TELEMETRY the compiler contacts bend-lang.com once a
      # day and writes ~/.bend. Orbit's package and the dev shell share this.
      bendBuildEnv = {
        BEND_SOURCE_DIR = bend;
        BEND_CLANG = "${pkgs.clang}/bin/clang";
        BEND_NO_TELEMETRY = "1";
      };
      # Orbit's host and its Bend worker (orbit/docs/HOST.md), which
      # orbit/build.rs compiles. The unit tests run here; the real-PTY test
      # binary, whose checks are ignored here, goes to the tests output for the
      # real-PTY checks. tests/lifecycle.rs needs host /usr/bin/setsid, which
      # the build sandbox lacks, so eon-test runs it.
      orbitPackage =
        assert ghostty.rev == "a887df42c56f6de86c0fe6da9c4eeca37931e083";
        assert bend.rev == "234cb9b950800f20b15feafcb4ce2f43e76e7fae";
        pkgs.rustPlatform.buildRustPackage {
          pname = workspacePackages.orbit;
          inherit (orbitIdentity) version;
          src = crateSource;
          cargoLock = workspaceCargoLock;
          outputs = [
            "out"
            "tests"
          ];
          cargoBuildFlags = [
            "--package"
            workspacePackages.orbit
          ];
          cargoTestFlags = [
            "--package"
            workspacePackages.orbit
            "--lib"
            "--test"
            "real_pty"
          ];
          nativeBuildInputs = [
            pkgs.bun
            pkgs.ncurses
            pkgs.zig_0_15.hook
          ];
          inherit (bendBuildEnv) BEND_SOURCE_DIR BEND_CLANG BEND_NO_TELEMETRY;
          inherit (ghosttyBuildEnv) LIBGHOSTTY_VT_SYS_CPU GHOSTTY_SOURCE_DIR GHOSTTY_ZIG_SYSTEM_DIR;
          dontUseZigBuild = true;
          dontUseZigCheck = true;
          dontUseZigInstall = true;
          postInstall = ''
            mapfile -t workers < <(find target -type f -path '*/release/build/*/out/yazelix-orbit-worker')
            mapfile -t realPty < <(find target -type f -path '*/release/deps/real_pty-*' -perm -u+x)
            test ''${#workers[@]} = 1
            test ''${#realPty[@]} = 1
            install -Dm755 "''${workers[0]}" "$out/libexec/yazelix-orbit/yazelix-orbit-worker"
            install -Dm755 "''${realPty[0]}" "$tests/libexec/real-pty"
            mkdir -p "$out/share/terminfo"
            ${pkgs.ncurses}/bin/tic -x -o "$out/share/terminfo" ${./orbit/terminfo/eon.terminfo}
            install -Dm444 ${./orbit/LICENSE} "$out/share/licenses/orbit/LICENSE"
            install -Dm444 ${ghostty}/LICENSE "$out/share/licenses/orbit/ghostty/LICENSE"
            install -Dm444 ${./licenses/libghostty-rs-MIT} "$out/share/licenses/orbit/libghostty-rs/LICENSE"
            install -Dm444 ${bend}/LICENSE "$out/share/licenses/orbit/bend/LICENSE"
          '';
        };
      orbitChecks = import ./nix/orbit-checks.nix {
        inherit pkgs bend orbitPackage;
      };

      venusRuntimeLibraries = [
        pkgs.libxkbcommon
        pkgs.vulkan-loader
        pkgs.wayland
      ];
      venusBuildInputs = [
        pkgs.fontconfig
        pkgs.freetype
        pkgs.libx11
        pkgs.libxcursor
        pkgs.libxi
        pkgs.libxrandr
        pkgs.libxkbcommon
        pkgs.wayland
      ];
      # Font directories are relative to this file, so the relocated bundle finds them too.
      venusFontsConf = pkgs.runCommand "fonts.conf" { } ''
        sed 's|<dir>${builtins.storeDir}/|<dir prefix="relative">|' ${
          pkgs.makeFontsConf {
            fontDirectories = [
              pkgs.dejavu_fonts
              pkgs.nerd-fonts.symbols-only
              anima.packages.${system}.accelerando-font
            ];
          }
        } > "$out"
        if grep -F '${builtins.storeDir}/' "$out"; then
          exit 1
        elif test $? != 1; then
          exit 1
        fi
      '';
      venusEnvironment = {
        set.FONTCONFIG_FILE = "${venusFontsConf}";
        prefix = {
          LD_LIBRARY_PATH = map (package: "${lib.getLib package}/lib") venusRuntimeLibraries;
          XDG_DATA_DIRS = [ "${pkgs.mesa}/share" ];
        };
      };
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
          buildInputs = venusBuildInputs;
          FONTCONFIG_FILE = venusFontsConf;
          postInstall = ''
            install -Dm444 ${./venus/LICENSE} "$out/share/licenses/venus/LICENSE"
            install -Dm444 ${./venus/THIRD_PARTY_NOTICES.md} "$out/share/licenses/venus/THIRD_PARTY_NOTICES.md"
          '';
          postFixup = ''
            wrapProgram "$out/bin/yazelix-venus" ${wrapperArgs venusEnvironment}
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
      starshipPackage = managedPackage starshipIdentity pkgs.starship;
      # zoxide runs the fzf that Eon puts first on the terminal PATH, not a compiled-in store
      # path that the relocated bundle cannot reach.
      zoxidePackage = managedPackage zoxideIdentity (pkgs.zoxide.override { withFzf = false; });
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
          ;
      };

      bashLicense = pkgs.runCommand "bash-license" { } ''
        ${pkgs.gnutar}/bin/tar -xOf ${bashPackage.src} bash-5.3/COPYING > "$out"
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

      # The Files popup's Yazi: Nova's F+ keys (Yazelix Nova defaults/yazi at cd95fee1)
      # without its editor actions.
      filesPopupConfig = pkgs.runCommand "eon-files-popup-config" { } ''
        mkdir -p "$out/plugins/quick-search.yazi" "$out/plugins/tab-folder.yazi"
        cat > "$out/keymap.toml" <<'EOF'
        [mgr]
        prepend_keymap = [
          { on = "<Tab>", run = "plugin quick-search", desc = "Search recent folders" },
          { on = "Z", run = "plugin quick-search", desc = "Search recent folders" },
          { on = "<BackTab>", run = "spot", desc = "Spot hovered file" },
          { on = "<S-Tab>", run = "spot", desc = "Spot hovered file" },
          { on = "<A-Enter>", run = "plugin tab-folder", desc = "Set the hovered or current folder as the tab folder" },
        ]
        EOF
        cat > "$out/init.lua" <<'EOF'
        require("zoxide"):setup({ update_db = true })
        for id = 1, 6 do
          Status:children_remove(id, id <= 3 and Status.LEFT or Status.RIGHT)
        end
        Status:children_add(function()
          return tostring(cx.layer) == "mgr" and " Alt+Enter Set tab folder · Tab/Z Search · Shift+Tab Spot" or ""
        end, 1000, Status.LEFT)
        EOF
        cat > "$out/plugins/quick-search.yazi/main.lua" <<'EOF'
        -- One search of Zoxide history for Tab and Z: Enter moves Yazi, Alt+Enter sets the tab folder.
        local M = {}

        local cwd = ya.sync(function() return tostring(cx.active.current.cwd) end)

        local function fail(content)
          ya.notify({ title = "Quick search", content = tostring(content), timeout = 5, level = "error" })
        end

        function M:entry()
          local history, err = Command("zoxide"):arg({ "query", "--list", "--exclude", cwd() }):output()
          if not history then return fail(err) end
          if not history.status.success then return fail(history.stderr) end
          if history.stdout == "" then return end

          local permit = ui.hide()
          local child, spawn_err = Command("fzf")
            :env("FZF_DEFAULT_OPTS", "--exact --no-sort --cycle --keep-right --info=inline --layout=reverse "
              .. "--height=100% --border=none --tabstop=1 --exit-0 --expect=alt-enter "
              .. "--bind=enter:accept-non-empty,ctrl-z:ignore,tab:abort,btab:up --prompt='Go to folder > ' "
              .. "--footer='Enter Go here · Alt+Enter Set tab folder · Tab/Esc Browse Yazi' "
              .. "--footer-border=none --color=footer:-1")
            :env("FZF_DEFAULT_OPTS_FILE", "")
            :stdin(Command.PIPED)
            :stdout(Command.PIPED)
            :stderr(Command.PIPED)
            :spawn()
          if child then
            child:write_all(history.stdout)
            child:flush()
          end
          local output, wait_err
          if child then output, wait_err = child:wait_with_output() end
          permit:drop()
          if not output then return fail(spawn_err or wait_err) end
          -- 130 is a cancel; 1 is Alt+Enter with no match.
          if output.status.code == 130 or output.status.code == 1 then return end
          if not output.status.success then return fail(output.stderr) end
          local key, target = output.stdout:match("^([^\n]*)\n(.*)")
          target = target and target:gsub("\n$", "")
          if not target or target == "" then return end
          if key == "alt-enter" then return require("tab-folder").set(target) end
          ya.emit("cd", { target, raw = true })
        end

        return M
        EOF
        cat > "$out/plugins/tab-folder.yazi/main.lua" <<'EOF'
        -- Alt+Enter: set this popup's Eon tab directory without moving Yazi.
        local M = {}

        local target = ya.sync(function()
          local current = cx.active.current
          local hovered = current.hovered
          return tostring(hovered and hovered.cha.is_dir and hovered.url or current.cwd)
        end)

        local function notify(content, level)
          ya.notify({ title = "Tab folder", content = content, timeout = level == "info" and 3 or 5, level = level })
        end

        -- After a supervisor recovery EON_TAB can name another tab, so act only
        -- while that tab still lists this terminal among its popups.
        local function owns(eon, tab)
          local session = (os.getenv("EON_TERMINAL") or ""):match("^([^.]+)%.")
          if not session then return false end
          local output, err = Command(eon):arg({ "workspace", "--json" }):output()
          if not output then return nil, tostring(err) end
          if not output.status.success then return nil, (output.stderr:gsub("%s+$", "")) end
          local ok, snapshot = pcall(ya.json_decode, output.stdout)
          if not ok or type(snapshot) ~= "table" then return false end
          for _, item in ipairs(snapshot.tabs or {}) do
            if item.id == tab then
              for _, popup in ipairs(item.popups or {}) do
                if popup.session == session then return true end
              end
            end
          end
          return false
        end

        function M.set(dir)
          local eon, tab = os.getenv("EON_FILES_CLI"), os.getenv("EON_TAB")
          if not eon or eon == "" or not tab or tab == "" then
            return notify("This Yazi is not an Eon Files popup", "error")
          end
          local owned, failure = owns(eon, tab)
          if owned == nil then
            return notify(failure ~= "" and failure or "eon workspace failed", "error")
          end
          if not owned then return notify("This Yazi is no longer a popup of tab " .. tab, "error") end
          local output, err = Command(eon):arg({ "tab", "directory", tab, "--", dir }):output()
          if not output then return notify(tostring(err), "error") end
          if not output.status.success then
            local detail = output.stderr:gsub("%s+$", "")
            return notify(detail ~= "" and detail or "eon exited with status " .. tostring(output.status.code), "error")
          end
          notify("Set to " .. dir, "info")
        end

        function M:entry() M.set(target()) end

        return M
        EOF
      '';

      eonEnvironment = out: {
        set = {
          EON_ORBIT = "${orbitPackage}/bin/yazelix-orbit";
          EON_VENUS = "${venusPackage}/bin/yazelix-venus";
          EON_CODEXBAR = "${codexbarPackage}/bin/CodexBarCLI";
          EON_ANIMA = "${animaPackage}/bin/anima";
          EON_NU = "${nushellPackage}/bin/nu";
          EON_BASH = "${bashPackage}/bin/bash";
          EON_HX = "${helixPackage}/bin/hx";
          EON_YAZI = "${yaziPackage}/bin/yazi";
          EON_DIRECTORY_PICKER_CONFIG = "${directoryPickerConfig}";
          EON_FILES_CONFIG = "${filesPopupConfig}";
          EON_YA = "${yaziPackage}/bin/ya";
          EON_LAZYGIT = "${lazygitPackage}/bin/lazygit";
          EON_NU_VENDOR_AUTOLOAD = "${out}/libexec/eon/shell/nu";
          EON_BASH_RC = "${out}/libexec/eon/shell/eon.bashrc";
          EON_SESSION_BIN = "${out}/libexec/eon/bin";
        };
        prefix.TERMINFO_DIRS = [ "${orbitPackage}/share/terminfo" ];
      };
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
            for command in nu bash hx yazi ya lazygit lg; do
              ln -s eon "$out/bin/eon-$command"
            done
            mkdir -p "$out/libexec/eon/bin"
            for command in nu bash hx yazi ya lazygit; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/$command"
            done
            for command in nu bash lazygit; do
              ln -s "../../../bin/eon-$command" "$out/libexec/eon/bin/eon-$command"
            done
            ln -s ${starshipPackage}/bin/starship "$out/libexec/eon/bin/starship"
            ln -s ${zoxidePackage}/bin/zoxide "$out/libexec/eon/bin/zoxide"
            ln -s ${fzfPackage}/bin/fzf "$out/libexec/eon/bin/fzf"
            ln -s ${animaPackage}/bin/anima "$out/libexec/eon/bin/anima"
            ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-directory-picker"
            ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-files"
            ln -s ${atuinPackage}/bin/atuin "$out/libexec/eon/bin/atuin"
            ln -s ${carapacePackage}/bin/carapace "$out/libexec/eon/bin/carapace"
            cp -r ${shellEnvironment} "$out/libexec/eon/shell"
            install -Dm444 ${./LICENSE} "$out/share/licenses/eon/LICENSE"
            install -Dm444 ${./runtime/LICENSE} "$out/share/licenses/eon/eon-runtime/LICENSE"
            install -Dm444 ${./runtime/NOTICE} "$out/share/licenses/eon/eon-runtime/NOTICE"
            install -Dm444 ${anima}/LICENSE "$out/share/licenses/eon/anima/LICENSE"
            install -Dm444 ${nushellPackage.src}/LICENSE "$out/share/licenses/eon/nushell/LICENSE"
            install -Dm444 ${bashLicense} "$out/share/licenses/eon/bash/COPYING"
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
            wrapProgram "$out/bin/eon" ${wrapperArgs (eonEnvironment (placeholder "out"))}
          '';
          meta = {
            description = "Native terminal workspaces with persistent terminals";
            license = lib.licenses.asl20;
            mainProgram = "eon";
            platforms = [ system ];
          };
        };
      eonbasePackage =
        assert manifest.composition.services == [ orbitIdentity.id ];
        assert manifest.composition.clients == [ venusIdentity.id ];
        eonPackage.overrideAttrs (_: {
          pname = "eonbase";
          desktopItems = [ ];
          postInstall = ''
            mv "$out/bin/eon" "$out/bin/eonbase"
            install -Dm444 ${./LICENSE} "$out/share/licenses/eonbase/LICENSE"
            install -Dm444 ${./runtime/LICENSE} "$out/share/licenses/eonbase/eon-runtime/LICENSE"
            install -Dm444 ${./runtime/NOTICE} "$out/share/licenses/eonbase/eon-runtime/NOTICE"
          '';
          postFixup = ''
            wrapProgram "$out/bin/eonbase" \
              --set EON_ORBIT "${orbitPackage}/bin/yazelix-orbit" \
              --set EON_VENUS "${venusPackage}/bin/yazelix-venus" \
              --unset EON_SESSION_BIN \
              --prefix TERMINFO_DIRS : "${orbitPackage}/share/terminfo"
          '';
          meta = {
            description = "Reusable Eon terminal product";
            license = lib.licenses.asl20;
            mainProgram = "eonbase";
            platforms = [ system ];
          };
        });
      eonbaseClosure = pkgs.closureInfo { rootPaths = [ eonbasePackage ]; };
      coinstalledProducts = pkgs.buildEnv {
        name = "eon-products-profile-check";
        paths = [ eonPackage eonbasePackage ];
      };
      eonbaseClosureCheck = pkgs.runCommand "eonbase-closure-check" { } ''
        test -x ${coinstalledProducts}/bin/eon
        test -x ${coinstalledProducts}/bin/eonbase
        for required in ${orbitPackage} ${venusPackage}; do
          ${pkgs.gnugrep}/bin/grep -Fx "$required" ${eonbaseClosure}/store-paths
        done
        for forbidden in \
          ${eonPackage} \
          ${codexbarPackage} \
          ${animaPackage} \
          ${nushellPackage} ${bashPackage} \
          ${starshipPackage} ${zoxidePackage} ${atuinPackage} ${carapacePackage} \
          ${fzfPackage} ${helixPackage} ${yaziPackage} ${lazygitPackage}; do
          if ${pkgs.gnugrep}/bin/grep -Fx "$forbidden" ${eonbaseClosure}/store-paths; then
            exit 1
          elif test $? != 1; then
            exit 1
          fi
        done
        test ! -e ${eonbasePackage}/libexec/eon
        ${pkgs.gnugrep}/bin/grep -F 'unset EON_SESSION_BIN' ${eonbasePackage}/bin/eonbase
        if ${pkgs.gnugrep}/bin/grep -F '/libexec/eon/bin' ${eonbasePackage}/bin/eonbase; then
          exit 1
        elif test $? != 1; then
          exit 1
        fi
        ${pkgs.coreutils}/bin/touch "$out"
      '';
      managedCommandPathCheck = pkgs.runCommand "eon-managed-command-path-check" { } ''
        export HOME="$TMPDIR" EON_CONFIG_HOME="$TMPDIR/config" PATH=${eonPackage}/libexec/eon/bin
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
      recordDemo = kinestra.lib.${system}.mkRecorder {
        name = "record-eon-demo";
        recipe = ./demo/record.rs;
        runtimeInputs = [ pkgs.swaybg ];
        environment = {
          EON_BIN = "${eonPackage}/bin/eon";
          EONBASE_BIN = "${eonbasePackage}/bin/eonbase";
          ZOXIDE_BIN = "${zoxidePackage}/bin/zoxide";
          EON_DEMO_PATH = lib.makeBinPath [ pkgs.coreutils pkgs.bash pkgs.git ];
        };
      };
      bundleRelink = import ./nix/bundle-relink.nix {
        inherit pkgs eonPackage wrapperArgs;
        wrappers = [
          {
            program = "${eonPackage}/bin/eon";
            environment = eonEnvironment "${eonPackage}";
            # Bash's ncurses for terminal types other than Orbit's.
            relocated.prefix.TERMINFO_DIRS = [ "${pkgs.ncurses}/share/terminfo" ];
          }
          {
            program = "${venusPackage}/bin/yazelix-venus";
            environment = venusEnvironment;
            # libxkbcommon's keymap and Compose data.
            relocated.setDefault = {
              XKB_CONFIG_ROOT = "${pkgs.xkeyboard_config}/etc/X11/xkb";
              XLOCALEDIR = "${pkgs.libx11}/share/X11/locale";
            };
            # The host's GPU driver shares Venus's gcc runtime, so both come from the host.
            hostRuntime = true;
          }
          {
            program = "${codexbarPackage}/bin/CodexBarCLI";
            environment = codexbarEnvironment;
          }
        ];
        fallbacks = [
          {
            link = "${helixPackage}/bin/runtime";
            target = helixPackage.HELIX_DEFAULT_RUNTIME;
          }
        ];
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
        eonbase = eonbasePackage;
        installer = import ./nix/installer.nix { inherit pkgs; };
        manual-bundle = import ./nix/linux-bundle.nix {
          inherit pkgs eonPackage;
          relink = bundleRelink;
          manifestFile = ./components/eon-v4.json;
          lockFile = ./flake.lock;
        };
      };
      apps.${system} = {
        default = {
          type = "app";
          program = "${eonPackage}/bin/eon";
        };
        eonbase = {
          type = "app";
          program = "${eonbasePackage}/bin/eonbase";
        };
        record-demo = {
          type = "app";
          program = "${recordDemo}/bin/record-eon-demo";
        };
      };
      checks.${system} = {
        default = eonPackage;
        helix-grammars = helix.checks.${system}.codeberg_grammars;
        eonbase-closure = eonbaseClosureCheck;
        managed-command-path = managedCommandPathCheck;
        bundle-relink = bundleRelink.tree;
        bundle-launch = bundleRelink.launch;
      } // orbitChecks;
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.cargo
          pkgs.rustc
          pkgs.clippy
          pkgs.rustfmt
          pkgs.git
          pkgs.ncurses
          pkgs.pkg-config
          pkgs.zig_0_15
          pkgs.bun
          (import ./nix/test-sandbox.nix { inherit pkgs; })
          br
          bv
        ];
        buildInputs = venusBuildInputs;
        inherit (ghosttyBuildEnv) LIBGHOSTTY_VT_SYS_CPU GHOSTTY_SOURCE_DIR GHOSTTY_ZIG_SYSTEM_DIR;
        inherit (bendBuildEnv) BEND_SOURCE_DIR BEND_CLANG BEND_NO_TELEMETRY;
        FONTCONFIG_FILE = venusFontsConf;
      };
    };
}
