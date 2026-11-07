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
      url = "github:Yazelix/nova-helix/af9acb1335d4e0b15f9f5725327afe79e94edee5";
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
    # Ghostty >= 8c425a7bd needs Zig 0.17, which only this newer nixpkgs
    # carries; it supplies Orbit's Zig and nothing else (eon-a9tr.1).
    nixpkgs-zig.url = "github:NixOS/nixpkgs/02c087f3fe673c9e317f206a627cb6b910336dbe";
    ghostty = {
      url = "github:ghostty-org/ghostty/521cf2c5d30bd48028ca61546c64e7913764073d";
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
      nixpkgs-zig,
      anima,
      helix,
      yazi,
      ratconfig,
      ghostty,
      bend,
      kinestra,
    }:
    let
      inherit (nixpkgs) lib;
      manifest = builtins.fromJSON (builtins.readFile ./components/eon-v5.json);
      # Each target the manifest names gets its outputs from this one definition.
      perSystem =
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          zig = nixpkgs-zig.legacyPackages.${system}.zig_0_17;
          eonCargo = builtins.fromTOML (builtins.readFile ./eon/crates/eon/Cargo.toml);
          helixCargo = builtins.fromTOML (builtins.readFile "${helix}/Cargo.toml");
          component =
            id:
            let
              matches = builtins.filter (candidate: candidate.id == id) manifest.components;
            in
            if builtins.length matches == 1 && builtins.elem system (builtins.head matches).targets then
              builtins.head matches
            else
              throw "Eon manifest must contain exactly one ${id} component for ${system}";
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
          # in the graph and its agent skill, and digests its Nix recipes into its
          # generation identity.
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
              ./docs/CONFIGURATION.md
              ./eon/skills/eon/SKILL.md
              ./nix/shell-environment.nix
            ];
          };
          workspaceCargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes."dpi-0.1.1" = "sha256-fJQqcXhKPGtivKo1AyyGhy1oNebTFGyzWDmOesFsltI=";
            outputHashes."libghostty-vt-0.2.1" = "sha256-Zojd1ziCw9+tbhmGYaFHFPlaxgbNbtmVBR99Otmg+o0=";
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
              # Both archives come from the same release with the same layout.
              src = pkgs.fetchurl {
                x86_64-linux = {
                  url = "https://github.com/steipete/CodexBar/releases/download/v${codexbarIdentity.version}/CodexBarCLI-v${codexbarIdentity.version}-linux-musl-x86_64.tar.gz";
                  hash = "sha256-oC26jOydDYucyteY5LxuonAbfIAcud1z9vzf7b5QfcY=";
                };
                aarch64-darwin = {
                  url = "https://github.com/steipete/CodexBar/releases/download/v${codexbarIdentity.version}/CodexBarCLI-v${codexbarIdentity.version}-macos-arm64.tar.gz";
                  hash = "sha256-nblQWO3ciCQdIoUVowr2hoygZ7qaJkUuRkLvWeL1T1I=";
                };
              }.${system};
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

          ghosttyDepsArchives = zig.fetchDeps {
            pname = "ghostty";
            version = "0-unstable";
            src = ghostty;
            fetchAll = true;
            hash = "sha256-Q0ZgKPVtXbiTbJqrMlGPJBnNpmaXNLUAk0VcqLBFGFI=";
          };
          # Zig 0.17 keeps fetched packages as archives, but `zig build --system`
          # wants them unpacked; each archive holds its package directory.
          ghosttyDeps = pkgs.runCommand "ghostty-zig-system-deps" { } ''
            mkdir "$out"
            for archive in ${ghosttyDepsArchives}/*.tar.gz; do
              tar -xzf "$archive" -C "$out"
            done
          '';
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
          # macOS's system ncurses reads only the legacy terminfo format, whose
          # numbers stop at 0x7fff, under hex directory names, so Darwin's entry
          # caps xterm-256color's pairs#0x10000 and adds 65 -> e, as kitty's does
          # (ORB-C10). orbit-terminfo-darwin inspects it on Linux.
          darwinTerminfo =
            let
              source = builtins.readFile ./orbit/terminfo/eon.terminfo;
              parent = "use=xterm-256color,";
              capped = builtins.replaceStrings [ parent ] [ "pairs#0x7fff, ${parent}" ] source;
            in
            assert lib.hasInfix parent source;
            pkgs.runCommand "eon-terminfo-darwin" { } ''
              ${pkgs.ncurses}/bin/tic -x -o terminfo ${builtins.toFile "eon.terminfo" capped}
              install -Dm444 terminfo/*/eon "$out/share/terminfo/e/eon"
              ln -s e "$out/share/terminfo/65"
            '';
          # Orbit's host and its Bend worker (orbit/docs/HOST.md), which
          # orbit/build.rs compiles. The unit tests run here; the real-PTY test
          # binary, whose checks are ignored here, goes to the tests output for the
          # real-PTY checks. tests/lifecycle.rs needs host /usr/bin/setsid, which
          # the build sandbox lacks, so eon-test runs it.
          orbitPackage =
            assert ghostty.rev == "521cf2c5d30bd48028ca61546c64e7913764073d";
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
                zig.hook
              ]
              # Ghostty runs Apple's libtool and xcrun nmedit, absent from Darwin stdenv.
              ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.cctools ];
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
                ${
                  if pkgs.stdenv.hostPlatform.isDarwin then
                    ''cp -R ${darwinTerminfo}/share/terminfo/. "$out/share/terminfo"''
                  else
                    ''${pkgs.ncurses}/bin/tic -x -o "$out/share/terminfo" ${./orbit/terminfo/eon.terminfo}''
                }
                install -Dm444 ${./orbit/LICENSE} "$out/share/licenses/orbit/LICENSE"
                install -Dm444 ${ghostty}/LICENSE "$out/share/licenses/orbit/ghostty/LICENSE"
                install -Dm444 ${./licenses/libghostty-rs-MIT} "$out/share/licenses/orbit/libghostty-rs/LICENSE"
                install -Dm444 ${bend}/LICENSE "$out/share/licenses/orbit/bend/LICENSE"
              '';
            };
          orbitChecks = import ./nix/orbit-checks.nix {
            inherit pkgs bend orbitPackage darwinTerminfo;
          };

          venusRuntimeLibraries = [
            pkgs.libxkbcommon
            pkgs.vulkan-loader
            pkgs.wayland
          ];
          venusBuildInputs = lib.optionals pkgs.stdenv.isLinux [
            pkgs.fontconfig
            pkgs.freetype
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxi
            pkgs.libxrandr
            pkgs.libxkbcommon
            pkgs.wayland
          ];
          venusFonts = [
            pkgs.dejavu_fonts
            pkgs.nerd-fonts.symbols-only
            anima.packages.${system}.accelerando-font
          ] ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.noto-fonts-color-emoji ];
          # Font directories are relative to this file, so the relocated bundle finds them too.
          venusFontsConf = pkgs.runCommand "fonts.conf" { } ''
            sed 's|<dir>${builtins.storeDir}/|<dir prefix="relative">|' ${
              pkgs.makeFontsConf {
                fontDirectories = venusFonts;
              }
            } > "$out"
            if grep -F '${builtins.storeDir}/' "$out"; then
              exit 1
            elif test $? != 1; then
              exit 1
            fi
          '';
          # Venus reads Eon's fonts through fontconfig on Linux. On macOS its
          # font database ignores fontconfig and reads EON_VENUS_FONT_DIRS.
          venusFontEnvironment =
            if pkgs.stdenv.isLinux then
              { FONTCONFIG_FILE = "${venusFontsConf}"; }
            else
              { EON_VENUS_FONT_DIRS = lib.concatStringsSep ":" venusFonts; };
          # Venus's hint fit test measures this exact packaged file, not whichever
          # DejaVu Sans the host also has.
          venusTestEnvironment = {
            EON_VENUS_TEST_SANS_FONT = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf";
          };
          venusEnvironment = {
            set = venusFontEnvironment;
            prefix = lib.optionalAttrs pkgs.stdenv.isLinux {
              LD_LIBRARY_PATH = map (package: "${lib.getLib package}/lib") venusRuntimeLibraries;
              XDG_DATA_DIRS = [ "${pkgs.mesa}/share" ];
            };
          };
          venusPackage =
            pkgs.rustPlatform.buildRustPackage ({
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
              postInstall = ''
                install -Dm444 ${./venus/LICENSE} "$out/share/licenses/venus/LICENSE"
                install -Dm444 ${./venus/THIRD_PARTY_NOTICES.md} "$out/share/licenses/venus/THIRD_PARTY_NOTICES.md"
              '';
              postFixup = ''
                wrapProgram "$out/bin/yazelix-venus" ${wrapperArgs venusEnvironment}
              '';
            }
            // venusFontEnvironment
            // venusTestEnvironment
            // lib.optionalAttrs pkgs.stdenv.isDarwin {
              # Venus's tests build their font database without
              # EON_VENUS_FONT_DIRS, and on macOS it reads only the system and
              # user font folders, so the tests get a user folder of Eon's fonts.
              preCheck = ''
                export HOME="$TMPDIR/home"
                mkdir -p "$HOME/Library/Fonts"
                ln -s ${lib.concatStringsSep " " venusFonts} "$HOME/Library/Fonts/"
              '';
            });

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

          # The Files popup's Yazi and its quick search: the EON-C18 pair, which a pending tab
          # opens on quick search, with files opening in a new editor pane. Nothing writes
          # Zoxide history; Eon records a pick. The runtime switches screens through
          # `ya emit-to` and the fzf socket in $EON_PAIR_SOCKET.
          filesPopupConfig = pkgs.runCommand "eon-files-popup-config" { } ''
            mkdir -p "$out/plugins/quick-search.yazi" "$out/plugins/tab-folder.yazi" "$out/plugins/smart-enter.yazi" "$out/plugins/close.yazi"
            cat > "$out/yazi.toml" <<'EOF'
            # Text files open through `eon edit`; a failure stays on screen until Enter.
            [opener]
            edit = [
              { run = '"$EON_FILES_CLI" edit -- %s1 >/dev/null || { printf "Press Enter to return to Files"; read _; }', block = true, desc = "Edit in a new editor pane", for = "unix" },
            ]
            EOF
            cat > "$out/keymap.toml" <<'EOF'
            [mgr]
            prepend_keymap = [
              { on = "<Enter>", run = "plugin smart-enter", desc = "Enter the hovered folder, else open as usual" },
              { on = "<Tab>", run = "plugin quick-search", desc = "Search recent folders" },
              { on = "<BackTab>", run = "spot", desc = "Spot hovered file" },
              { on = "<S-Tab>", run = "spot", desc = "Spot hovered file" },
              { on = "<A-Enter>", run = "plugin tab-folder", desc = "Use the hovered or current folder for the tab" },
              { on = "<Esc>", run = "plugin close", desc = "Leave a selection, filter or find, else close" },
              { on = "<C-[>", run = "plugin close", desc = "Leave a selection, filter or find, else close" },
              { on = "<C-c>", run = "plugin close", desc = "Leave a selection, filter or find, else close" },
              # Files has one search and one close.
              { on = "Z", run = "noop" },
              { on = "q", run = "noop" },
              { on = "Q", run = "noop" },
            ]
            EOF
            cat > "$out/init.lua" <<'EOF'
            -- A pending tab's pair, and the pair Alt+Z opens, start on quick search.
            if os.getenv("EON_FILES_START") == "quick" then ya.emit("plugin", { "quick-search" }) end
            for id = 1, 6 do
              Status:children_remove(id, id <= 3 and Status.LEFT or Status.RIGHT)
            end
            Status:children_add(function()
              return tostring(cx.layer) == "mgr" and " Enter Open · Alt+Enter Use folder · Tab Search · Esc Close" or ""
            end, 1000, Status.LEFT)
            EOF
            cat > "$out/plugins/smart-enter.yazi/main.lua" <<'EOF'
            --- @sync entry
            -- Yazi's open rules would edit a folder; Enter moves into a hovered one
            -- instead and otherwise opens as usual (after yazi-rs/plugins smart-enter).
            return {
              entry = function()
                local hovered = cx.active.current.hovered
                ya.emit(hovered and hovered.cha.is_dir and "enter" or "open", {})
              end,
            }
            EOF
            cat > "$out/plugins/quick-search.yazi/main.lua" <<'EOF'
            -- Quick search over Zoxide history: Enter uses the highlighted folder, Tab moves
            -- Files there, and Esc or Ctrl+C closes Files.
            local M = {}

            local cwd = ya.sync(function() return tostring(cx.active.current.cwd) end)

            -- The startup pair's notice of other running windows heads only its first quick
            -- search.
            local notice = ya.sync(function(state)
              if state.noticed then return nil end
              state.noticed = true
              local text = os.getenv("EON_FILES_NOTICE")
              return text ~= "" and text or nil
            end)

            local function fail(content)
              ya.notify({ title = "Quick search", content = tostring(content), timeout = 5, level = "error" })
            end

            -- One quick search at a time: Yazi would queue a second behind the shown one and
            -- show it when the first ends.
            local claim = ya.sync(function(state)
              if state.searching then return false end
              state.searching = true
              return true
            end)
            local release = ya.sync(function(state) state.searching = nil end)

            local function search()
              local history, err = Command("zoxide")
                :arg({ "query", "--list", "--exclude", cwd() })
                :stdout(Command.PIPED)
                :stderr(Command.NULL)
                :spawn()
              if not history then return fail(err) end

              -- The runtime finds a shown quick search at this socket, and asks it to end
              -- with status 11 to switch to Files.
              local arguments = {}
              local socket = os.getenv("EON_PAIR_SOCKET")
              if socket and socket ~= "" then arguments[#arguments + 1] = "--listen=" .. socket end
              local header = notice()
              if header then arguments[#arguments + 1] = "--header=" .. header end
              local permit = ui.hide()
              -- fzf reads the history as it arrives instead of waiting for all of it.
              local child, spawn_err = Command("fzf")
                :arg(arguments)
                :env("FZF_DEFAULT_OPTS", "--exact --no-sort --cycle --keep-right --info=inline --layout=reverse "
                  .. "--height=100% --no-clear --border=none --tabstop=1 --expect=tab "
                  .. "--bind=enter:accept-non-empty,ctrl-z:ignore,btab:up --prompt='Quick search > ' "
                  .. "--footer='Enter Use · Tab Browse from here · Esc Close' "
                  .. "--footer-border=none --color=footer:-1")
                :env("FZF_DEFAULT_OPTS_FILE", "")
                :stdin(history:take_stdout())
                :stdout(Command.PIPED)
                :stderr(Command.PIPED)
                :spawn()
              local output, wait_err
              if child then output, wait_err = child:wait_with_output() end
              -- fzf leaves its socket behind when it ends.
              if socket and socket ~= "" then os.remove(socket) end
              -- A query that already ended on its own and failed left fzf an empty list.
              local ended = history:try_wait()
              -- No exit needs the rest of the history, so stop and reap the query.
              history:start_kill()
              history:wait()
              local code = output and output.status.code
              local key, target
              if output and output.status.success then
                key, target = output.stdout:match("^([^\n]*)\n(.*)")
                target = target and target:gsub("\n$", "")
                if target == "" then target = nil end
              end
              -- A pick or Esc is reported while Yazi is still hidden: a pending tab's pair
              -- hides as its first pane starts, so fzf's kept last frame (--no-clear) goes
              -- straight to the pane, with no Files frame between.
              local reported, report_err = pcall(function()
                if code == 130 then
                  require("tab-folder").close()
                elseif target and key ~= "tab" then
                  require("tab-folder").set(target)
                end
              end)
              permit:drop()
              if not reported then fail(report_err) end
              if ended and not ended.success then
                fail("zoxide query exited with status " .. tostring(ended.code))
              end
              if not output then return fail(spawn_err or wait_err) end
              -- 1 is Tab with no result, and 11 the runtime's switch to Files: Files stays
              -- where it is.
              if code == 130 or code == 1 or code == 11 then return end
              if not output.status.success then return fail(output.stderr) end
              if target and key == "tab" then ya.emit("cd", { target, raw = true }) end
            end

            function M:entry()
              if not claim() then return end
              local ok, err = pcall(search)
              release()
              if not ok then fail(err) end
            end

            return M
            EOF
            cat > "$out/plugins/tab-folder.yazi/main.lua" <<'EOF'
            -- Alt+Enter: use the hovered or current folder for this popup's Eon tab without
            -- moving Yazi. Also the pick and close for quick search and Esc.
            local M = {}

            local target = ya.sync(function()
              local current = cx.active.current
              local hovered = current.hovered
              return tostring(hovered and hovered.cha.is_dir and hovered.url or current.cwd)
            end)

            local function notify(content, level)
              ya.notify({ title = "Tab folder", content = content, timeout = level == "info" and 3 or 5, level = level })
            end

            local function ran(output, err)
              if not output then return nil, tostring(err) end
              if output.status.success then return output end
              local detail = output.stderr:gsub("%s+$", "")
              return nil, detail ~= "" and detail or "eon exited with status " .. tostring(output.status.code)
            end

            -- This terminal's popup ID, false when a readable snapshot does not list it, or nil
            -- and why when the snapshot cannot be read. After a supervisor recovery EON_TAB can
            -- name another tab, so act only while that tab still lists this terminal among its
            -- popups.
            local function popup(eon, tab)
              local session = (os.getenv("EON_TERMINAL") or ""):match("^([^.]+)%.")
              if not session then return false end
              local output, failure = ran(Command(eon):arg({ "workspace", "--json" }):output())
              if not output then return nil, failure end
              local ok, snapshot = pcall(ya.json_decode, output.stdout)
              if not ok or type(snapshot) ~= "table" then
                return nil, "eon workspace --json returned no readable workspace"
              end
              for _, item in ipairs(snapshot.tabs or {}) do
                if item.id == tab then
                  for _, entry in ipairs(item.popups or {}) do
                    if entry.session == session then return entry.id end
                  end
                end
              end
              return false
            end

            -- Calls `eon ARGS...` for this popup's tab with its popup ID, or notifies why not. A
            -- Yazi that is no longer a listed popup calls `unlisted` instead, when given.
            local function act(arguments, unlisted)
              local eon, tab = os.getenv("EON_FILES_CLI"), os.getenv("EON_TAB")
              if not eon or eon == "" or not tab or tab == "" then
                if unlisted then return unlisted() end
                return notify("This Yazi is not an Eon Files popup", "error")
              end
              local id, failure = popup(eon, tab)
              if id == nil then return notify(failure ~= "" and failure or "eon workspace failed", "error") end
              if not id then
                if unlisted then return unlisted() end
                return notify("This Yazi is no longer a popup of tab " .. tab, "error")
              end
              local output, err = ran(Command(eon):arg(arguments(tab, id)):output())
              if not output then return notify(err, "error") end
              return true
            end

            function M.set(dir)
              if act(function(tab) return { "tab", "directory", tab, "--", dir } end) then
                notify("Set to " .. dir, "info")
              end
            end

            -- A Yazi its tab no longer lists, as after a supervisor recovery, has no popup to
            -- hide, so closing quits it.
            function M.close()
              act(function(_, id) return { "popup", "hide", id } end, function() ya.emit("quit", {}) end)
            end

            function M:entry() M.set(target()) end

            return M
            EOF
            cat > "$out/plugins/close.yazi/main.lua" <<'EOF'
            -- Esc and Ctrl+C: leave a visual mode, selection, filter, find or search
            -- first, as Yazi's escape does; otherwise close Files.
            local M = {}

            local busy = ya.sync(function()
              local tab = cx.active
              return tab.mode.is_visual or #tab.selected > 0 or tab.finder ~= nil
                or tab.current.files.filter ~= nil or tab.current.cwd.is_search
            end)

            function M:entry()
              if busy() then return ya.emit("escape", {}) end
              require("tab-folder").close()
            end

            return M
            EOF
          '';

          eonEnvironment = out: {
            set = {
              EON_ORBIT = "${orbitPackage}/bin/yazelix-orbit";
              EON_VENUS = "${venusPackage}/bin/yazelix-venus";
              EON_CODEXBAR = "${codexbarPackage}/bin/CodexBarCLI";
              EON_ANIMA = "${animaPackage}/bin/anima";
              EON_ICON = "${out}/share/eon/eon-icon-256.rgba";
              EON_NU = "${nushellPackage}/bin/nu";
              EON_BASH = "${bashPackage}/bin/bash";
              EON_HX = "${helixPackage}/bin/hx";
              EON_YAZI = "${yaziPackage}/bin/yazi";
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
              # Eon Runtime's lease tests run this Orbit, including the ignored one
              # that measures it under PTY load.
              ORBIT_BIN = "${orbitPackage}/bin/yazelix-orbit";
              checkFlags = [ "--include-ignored" ];
              preCheck =
                if pkgs.stdenv.isLinux then
                  ''
                    # Keep ephemeral lifecycle state off the builder's disk journal.
                    eonTestBuildTmpdir="$TMPDIR"
                    export TMPDIR=/dev/shm
                  ''
                else
                  ''
                    # macOS socket paths hold 103 bytes, so the tests' runtime
                    # roots need a TMPDIR of at most 7 bytes. Each build takes
                    # its own, since /tmp is shared by every build user.
                    eonTestBuildTmpdir="$TMPDIR"
                    eonTestTmpdir=
                    for name in {0..9} {a..z}; do
                      if mkdir -m 700 "/tmp/e$name" 2>/dev/null; then
                        eonTestTmpdir="/tmp/e$name"
                        export TMPDIR="$eonTestTmpdir"
                        break
                      fi
                    done
                    test "$TMPDIR" != "$eonTestBuildTmpdir"
                    eonTestCleanup() {
                      if [ -n "''${eonTestTmpdir:-}" ]; then
                        rm -rf "$eonTestTmpdir"
                      fi
                    }
                    # A failed check skips postCheck.
                    failureHooks+=(eonTestCleanup)
                  '';
              postCheck =
                lib.optionalString (!pkgs.stdenv.isLinux) ''
                  eonTestCleanup
                  eonTestTmpdir=
                ''
                + ''
                  export TMPDIR="$eonTestBuildTmpdir"
                '';
              nativeBuildInputs = [
                pkgs.copyDesktopItems
                pkgs.makeWrapper
              ];
              # Runtime tests paste printed commands into Eon's Nushell.
              nativeCheckInputs = [ nushellPackage ];
              desktopItems = [ desktopItem ];
              postInstall = ''
                for size in 16 24 32 48 64 128 256 512; do
                  icon_dir="$out/share/icons/hicolor/''${size}x''${size}/apps"
                  mkdir -p "$icon_dir"
                  ${pkgs.imagemagick}/bin/magick ${./assets/eon.png} \
                    -filter Box -resize "''${size}x''${size}" "$icon_dir/eon.png"
                done
                # The Eon Bar's icon: the mark in its own colours, centred in a 256x256 square,
                # as raw 8-bit RGBA with premultiplied alpha.
                mkdir -p "$out/share/eon"
                ${pkgs.imagemagick}/bin/magick ${./assets/eon.png} -trim +repage -background none \
                  -set option:side '%[fx:max(w,h)]' -gravity center -extent '%[side]x%[side]' \
                  -filter Box -resize 256x256 -channel RGB -fx 'u*u.a' +channel -depth 8 \
                  "rgba:$out/share/eon/eon-icon-256.rgba"
                test "$(stat -c %s "$out/share/eon/eon-icon-256.rgba")" = 262144
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
                # A terminal's own build answers `eon` there after the profile changes.
                ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon"
                ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-files"
                ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-commands"
                ln -s "../../../bin/eon" "$out/libexec/eon/bin/eon-sessions"
                ln -s ${atuinPackage}/bin/atuin "$out/libexec/eon/bin/atuin"
                ln -s ${carapacePackage}/bin/carapace "$out/libexec/eon/bin/carapace"
                cp -r ${shellEnvironment} "$out/libexec/eon/shell"
                install -Dm444 ${./LICENSE} "$out/share/licenses/eon/LICENSE"
                install -Dm444 ${./eon/skills/eon/SKILL.md} "$out/share/eon/skills/eon/SKILL.md"
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
          managedCommandPathCheck = import ./nix/managed-command-checks.nix {
            inherit pkgs eonPackage;
          };
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
          # The macOS proof harness (docs/DEVELOPMENT.md), a development tool
          # outside Eon's closure. eon-test runs its tests.
          macProof = pkgs.rustPlatform.buildRustPackage {
            pname = "mac-proof";
            inherit ((builtins.fromTOML (builtins.readFile ./eon/tools/mac-proof/Cargo.toml)).package) version;
            src = crateSource;
            cargoLock = workspaceCargoLock;
            cargoBuildFlags = [
              "--package"
              "mac-proof"
            ];
            doCheck = false;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postFixup = ''
              wrapProgram "$out/bin/mac-proof" --prefix PATH : ${lib.makeBinPath [ pkgs.git pkgs.gnutar ]}
            '';
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
              x86_64-linux = {
                url = "https://github.com/Dicklesworthstone/beads_rust/releases/download/v0.7.2/br-0.7.2-linux_musl_amd64.tar.gz";
                hash = "sha256-v6GvOaScpcGuj8ehPqOg05Y6qkuJEtcebDKVhHwJ12w=";
              };
              aarch64-darwin = {
                url = "https://github.com/Dicklesworthstone/beads_rust/releases/download/v0.7.2/br-0.7.2-darwin_arm64.tar.gz";
                hash = "sha256-kRcQATgRb7P2nUWxBMe0UJlx4s44+8xQGDHcg9AqE8Y=";
              };
            }.${system}} -C "$out/bin" br LICENSE
            mv "$out/bin/LICENSE" "$out/share/licenses/br/LICENSE"
          '';
          bv = pkgs.runCommand "bv-0.25.2" { } ''
            mkdir -p "$out/bin" "$out/share/licenses/bv"
            ${pkgs.gnutar}/bin/tar -xzf ${pkgs.fetchurl {
              x86_64-linux = {
                url = "https://github.com/Dicklesworthstone/beads_viewer/releases/download/v0.25.2/bv_0.25.2_linux_amd64.tar.gz";
                hash = "sha256-TCaQh696Dd72glYJy0zffbAWNiTSbXsCAww4wmCx4Zk=";
              };
              aarch64-darwin = {
                url = "https://github.com/Dicklesworthstone/beads_viewer/releases/download/v0.25.2/bv_0.25.2_darwin_arm64.tar.gz";
                hash = "sha256-jOuVH2WKdR92qzCzPt9p2xetvFPAtpptnCdAmT/TKlg=";
              };
            }.${system}} -C "$out/bin" bv LICENSE
            mv "$out/bin/LICENSE" "$out/share/licenses/bv/LICENSE"
          '';
        in
        {
          # The installer, the manual bundle, the release files, the demo recorder
          # and the bundle checks are Linux's; Darwin has none of them.
          packages.${system} = {
            default = eonPackage;
            eonbase = eonbasePackage;
            helix-grammar-sources = helix.packages.${system}.grammar-sources;
          } // lib.optionalAttrs pkgs.stdenv.isLinux rec {
            installer = import ./nix/installer.nix { inherit pkgs; };
            manual-bundle = import ./nix/linux-bundle.nix {
              inherit pkgs eonPackage;
              relink = bundleRelink;
              manifestFile = ./components/eon-v5.json;
              lockFile = ./flake.lock;
            };
            release = import ./nix/release.nix {
              inherit pkgs installer;
              bundle = manual-bundle;
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
            mac-proof = {
              type = "app";
              program = "${pkgs.writeShellScript "mac-proof" ''
                export MAC_PROOF_SOURCE=${self}
                export MAC_PROOF_REV=${self.rev or "dirty"}
                exec ${macProof}/bin/mac-proof "$@"
              ''}";
            };
          } // lib.optionalAttrs pkgs.stdenv.isLinux {
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
            # The pair's quick search against a slow stand-in fzf (EON-C18), and a pick or
            # Esc reported while Yazi is still hidden.
            quick-search-guard = pkgs.runCommand "eon-quick-search-guard" {
              nativeBuildInputs = [ pkgs.lua5_4 ];
            } ''
              lua ${./nix/quick-search-guard.lua} \
                ${filesPopupConfig}/plugins/quick-search.yazi/main.lua "$TMPDIR/terminal-2.quick.sock"
              touch "$out"
            '';
          } // lib.optionalAttrs pkgs.stdenv.isLinux {
            bundle-relink = bundleRelink.tree;
            bundle-launch = bundleRelink.launch;
          } // orbitChecks;
          devShells.${system}.default = pkgs.mkShell ({
            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.clippy
              pkgs.rustfmt
              pkgs.git
              pkgs.ncurses
              pkgs.pkg-config
              zig
              pkgs.bun
              (import ./nix/test-sandbox.nix { inherit pkgs; })
              # Runtime tests paste printed commands into Eon's Nushell.
              nushellPackage
              br
              bv
            ]
            # Orbit's build needs cctools here too, on Macs without Xcode's command line tools.
            ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.cctools ];
            buildInputs = venusBuildInputs;
            inherit (ghosttyBuildEnv) LIBGHOSTTY_VT_SYS_CPU GHOSTTY_SOURCE_DIR GHOSTTY_ZIG_SYSTEM_DIR;
            inherit (bendBuildEnv) BEND_SOURCE_DIR BEND_CLANG BEND_NO_TELEMETRY;
          } // venusFontEnvironment // venusTestEnvironment);
        };
    in
    lib.foldl' lib.recursiveUpdate { } (map perSystem manifest.product.targets);
}
