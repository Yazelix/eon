{ pkgs, manifest, noticeTool, bundleRelink, helix, yazi, ghostty, ghosttyDeps, bend, anima, nixpkgsSource
, zig, rustToolchain, system, githubSource, codexbarIdentity
, animaIdentity, helixIdentity, venusFonts, venusBuildInputs, venusRuntimeLibraries
, venusFontsConf, filesPopupConfig, eonRust, eonPackage, orbitPackage, venusPackage, installerPackage
, animaPackage, nushellPackage, bashPackage, starshipPackage, zoxidePackage, fzfPackage
, atuinPackage, carapacePackage, helixPackage, yaziPackage, lazygitPackage, codexbarPackage }:
let
  inherit (pkgs) lib;
  launcher = bundleRelink.launcher;
  noticeHashes = files: lib.listToAttrs (map ({ file, hash }: {
    name = builtins.unsafeDiscardStringContext "${file}";
    # Attribute names cannot retain the dependency context; hash values can.
    value = lib.addContextFrom "${file}" hash;
  }) files);
  auditExceptions = builtins.fromJSON (builtins.readFile ./audit-exceptions.json);
  native = import ./native.nix { inherit pkgs; };
  eonStaticInputs = [ "system-linker-runtime" "rust-standard-library" ];
  ghosttyNoticeCatalogue = "${ghostty}/deps/libghostty-vt-third-party.json";
  ghosttyNoticeEntries = map (dependency: {
    id = "ghostty-${dependency.name}";
    inherit (dependency) version;
    source = if dependency.commit == null then "${ghostty}/${dependency.path}" else
      "${dependency.url} (${dependency.commit})";
    # Explicit choices for the expressions in the upstream catalogue.
    licenses = {
      MIT = [ "MIT" ];
      "MIT AND Unicode-3.0" = [ "MIT" "Unicode-3.0" ];
      "Apache-2.0 OR MIT" = [ "MIT" ];
      "Apache-2.0 OR BSD-3-Clause" = [ "BSD-3-Clause" ];
    }.${dependency.license} or [ ];
    source_roots = [ (if dependency.kind == "vendored" then "${ghostty}/${dependency.path}" else
      "${ghosttyDeps}/${dependency.hash}") ];
    notice_files = lib.optionals (dependency.name == "uucode") (let
      upstream = pkgs.fetchFromGitHub {
        owner = "jacobsandlund";
        repo = "uucode";
        rev = dependency.commit;
        hash = "sha256-fZTYVFN0N10R0Ty+3tpMkTLqEBq4tTpkBK6G1m6VgmY=";
      };
    in [ "${upstream}/licenses/LICENSE_unicode" "${upstream}/licenses/LICENSE_Bjoern_Hoehrmann" ]);
  }) (builtins.filter (dependency: dependency.use == "linked")
    (builtins.fromJSON (builtins.readFile ghosttyNoticeCatalogue)).third_party);
  codexbarSource = githubSource codexbarIdentity "sha256-sndZwprigQ99PWNcVUixjKGO6rMrA3vB1oXI6As6aPY=";
  installerLock = builtins.fromTOML (builtins.readFile ../../installer/Cargo.lock);
  installerZstd = lib.findFirst (crate: crate.name == "zstd-sys")
    (throw "installer zstd source needs a notice review") installerLock.package;
  installerZstdSource = pkgs.fetchurl {
    url = "https://static.crates.io/crates/zstd-sys/zstd-sys-${installerZstd.version}.crate";
    sha256 = installerZstd.checksum;
  };
  installerZstdNotice = pkgs.runCommand "installer-zstd-license" {
    nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
  } ''
    tar -xOf ${installerZstdSource} zstd-sys-${installerZstd.version}/zstd/LICENSE > "$out"
  '';
  helixRuntimeInlineNotices = pkgs.runCommand "helix-runtime-inline-notices" { } ''
    mkdir "$out"
    cd ${helix}/runtime
    grep -rilZ -E '^[;#]+[[:space:]]*.*(copyright|licen[cs]e:|licensed under|SPDX-)' \
      --include='*.scm' --include='*.toml' queries themes > "$TMPDIR/matching"
    while IFS= read -r -d "" file; do
      install -D "$file" "$out/$file.license"
    done < "$TMPDIR/matching"
  '';
  apache2 = pkgs.fetchurl
    ((builtins.fromJSON (builtins.readFile ./standard-license-texts.json))."Apache-2.0");
  certdataNotice = pkgs.runCommand "nss-certdata-license-notice" { } ''
    sed -n '1,5p' ${pkgs.cacert.src} > "$out"
    grep -q 'Mozilla Public' "$out"
  '';
  mpl2 = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/MPL-2.0.txt";
    hash = "sha256-ZqMQfVrWoFiqt1PqrCBHzLLtDjlGXdD+WETaPjANUXI=";
  };
  originalCrateNotice = repository: revision: version: files: {
    inherit version revision;
    files = map (file: "${pkgs.fetchurl {
      url = "https://raw.githubusercontent.com/${repository}/${revision}/${file.name}";
      hash = file.hash;
    }}") files;
  };
  accesskitNotice = version: originalCrateNotice "AccessKit/accesskit"
    "f3026222559c7f5f46d8f25cb7404de68eb149f0" version [
      { name = "LICENSE-MIT"; hash = "sha256-I/GOA9xJ35FiL+KnYXZJdATkbO2KcV2dK2enRGVxzKM="; }
      { name = "LICENSE-APACHE"; hash = "sha256-Yseh419WQGiW16p8pS0MwNJyrAIrXSeW59aQXbijY2o="; }
    ];
  sourceBuildSupport = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../nix ../../relink ../../launcher ../../components
      ../../flake.nix ../../flake.lock ../../LICENSE
    ];
  };
  sourceName = "eon-${eonPackage.version}-x86_64-linux-sources.tar.zst";
  nixpkgsSourcePath = builtins.path { path = nixpkgsSource.outPath; name = "source"; };
  helixSourcePath = builtins.path { path = helix.outPath; name = "source"; };
  # MIT, chosen by Eon's maintainer; the record pins the exact Anima bytes it covers.
  accelerandoGrant = let
    grant = import ./accelerando-grant.nix { inherit lib; };
  in {
    inherit (grant) license;
    record = "${grant.path}";
    assets = noticeHashes (lib.mapAttrsToList (relative: hash: {
      file = "${anima}/${relative}";
      inherit hash;
    }) grant.assets);
  };
in
import ./default.nix {
  inherit pkgs manifest;
  sourceRelease = {
    filename = sourceName;
    releases_page = "https://github.com/Yazelix/eon/releases";
    support = {
      nixpkgs = nixpkgsSourcePath;
      eon = sourceBuildSupport;
      helix = helixSourcePath;
    };
    instructions = ./source-instructions.md;
    runtime_exception_basis = ''
      glibc ${pkgs.glibc.version} startup code in sysdeps/x86_64/start.S grants
      unlimited permission to link its compiled form into other programs and
      distribute those programs. csu/libc-start.c is in the host libc.so for these
      dynamically linked binaries; the static installer and launcher use musl.
      GCC ${pkgs.stdenv.cc.cc.version} crtstuff/libgcc code retains COPYING.RUNTIME
      (GCC Runtime Library Exception 3.1). The selected compilation processes use
      GPL-compatible GCC, rustc/LLVM and LLVM-based native toolchains without
      non-GPL-compatible GCC plugins or intermediate-code processing. This is the
      Eligible Compilation Process basis, not a relicensing of runtime sources.
      libquadmath remains LGPL and dynamically replaceable. The source, selected
      patches and original exceptions are included; no relink kit is promised for
      excepted startup objects. Static LGPL code without an applicable exception
      requires a separate decision before shipment.
    '';
  };
  nativeBindings = native.bindings;
  nativeExclusions = native.exclusions;
  tool = noticeTool;
  tree = bundleRelink.tree;
  inherit (bundleRelink) launcherPrograms;
  binaryInputs = {
    ${builtins.unsafeDiscardStringContext "${orbitPackage}/libexec/yazelix-orbit/yazelix-orbit-worker"} =
      [ "system-linker-runtime" "bend-worker" ];
  };
  grammarSources = helix.packages.${system}.grammar-sources;
  helixSource = helix;
  packages = [
    eonPackage eonRust orbitPackage venusPackage installerPackage launcher
    animaPackage nushellPackage bashPackage starshipPackage zoxidePackage fzfPackage
    atuinPackage carapacePackage helixPackage yaziPackage lazygitPackage codexbarPackage
    pkgs.bash pkgs.curlMinimal pkgs.cacert pkgs.bc pkgs.iana-etc pkgs.mailcap pkgs.tzdata
    pkgs.xkeyboard_config
    pkgs.dejavu_fonts.minimal pkgs.stdenv.cc.cc
  ] ++ venusFonts ++ venusBuildInputs ++ venusRuntimeLibraries;
  auditToolchain = { nixpkgs = toString nixpkgsSource; rust = toString rustToolchain; };
  rustProducers = map (producer: producer // {
    auditExceptions = lib.concatMap (exception: map (package: {
      inherit package;
      inherit (exception) root graph_sha256 reason evidence;
    }) exception.packages) (auditExceptions.${producer.id} or [ ]);
  }) [
    { id = "eon"; package = eonRust; roots = [ "eon" ]; manifest = "eon/crates/eon/Cargo.toml"; }
    { id = "orbit"; package = orbitPackage; roots = [ "yazelix-orbit" ]; manifest = "orbit/Cargo.toml"; }
    {
      id = "venus";
      package = venusPackage;
      roots = [ "yazelix-venus" ];
      manifest = "venus/Cargo.toml";
      crateNotices = {
        accesskit = accesskitNotice "0.25.2";
        accesskit_atspi_common = accesskitNotice "0.21.1";
        accesskit_consumer = accesskitNotice "0.39.2";
        accesskit_unix = accesskitNotice "0.24.1";
        accesskit_winit = accesskitNotice "0.34.2";
        harfrust = originalCrateNotice "harfbuzz/harfrust"
          "efdae3142ab76a2f1524d72cff9e3dfdc5afd7ca" "0.5.2" [
            { name = "LICENSE"; hash = "sha256-Onw/C4h6u3xjj68CLSkldBhFhhTGcksSDO7/snn5x9I="; }
          ];
        profiling = originalCrateNotice "aclysma/profiling"
          "8271551172eb6fa4cba47369aedd93790c623df9" "1.0.18" [
            { name = "LICENSE-MIT"; hash = "sha256-yBZ/3u7UbT8kTT+Fxb+ZjOiJNDaRwyvixhqLxLXAgzM="; }
          ];
        spirv = originalCrateNotice "gfx-rs/rspirv"
          "8afc3d0ac8e158128cd1410bb2e4b4c26ab11bb4" "0.4.0+sdk-1.4.341.0" [
            { name = "LICENSE"; hash = "sha256-z8d0m5b2O9McPEK1xHG/dWgUBT6EfBDz6wA0F7xSPTA="; }
          ];
      };
      sourceNotices = {
        "orbit/LICENSE" = ../../orbit/LICENSE;
        "runtime/LICENSE" = ../../runtime/LICENSE;
      };
    }
    { id = "anima"; package = animaPackage; roots = [ "yazelix_screen" ]; }
    { id = "nushell"; package = nushellPackage; roots = [ "nu" ]; nixpkgs = true; }
    { id = "starship"; package = starshipPackage; roots = [ "starship" ]; nixpkgs = true; }
    { id = "zoxide"; package = zoxidePackage; roots = [ "zoxide" ]; nixpkgs = true; }
    {
      id = "atuin";
      package = atuinPackage;
      roots = [ "atuin" "atuin-server" "atuin-nucleo-bench" ];
      manifest = "crates/atuin/Cargo.toml";
      nixpkgs = true;
      # The benchmark has no manifest license field; the repository's MIT terms cover it.
      sourceNotices."crates/atuin-nucleo/bench/LICENSE" = "${atuinPackage.src}/LICENSE";
    }
    { id = "helix"; package = helixPackage; roots = [ "helix-term" ]; manifest = "helix-term/Cargo.toml"; }
    { id = "yazi"; package = yaziPackage; roots = [ "yazi-fm" "yazi-cli" ]; source = yazi; nixpkgs = true; }
    { id = "installer"; package = installerPackage; roots = [ "eon-install" ]; target = "x86_64-unknown-linux-musl"; }
    {
      id = "launcher";
      package = launcher;
      roots = [ "eon-launcher" ];
      target = "x86_64-unknown-linux-musl";
      sourceNotices.LICENSE = "${../../LICENSE}";
    }
  ];
  goProducers = [
    { id = "fzf"; package = fzfPackage; binary = "fzf"; }
    { id = "carapace"; package = carapacePackage; binary = "carapace"; }
    { id = "lazygit"; package = lazygitPackage; binary = "lazygit"; }
  ];
  packageOverrides = {
    ${launcher.name} = {
      id = "archive-launcher";
      licenses = [ "Apache-2.0" ];
      notice_files = [ "${../../LICENSE}" ];
      embedded = [ "rust-nixpkgs-standard-library" "musl-runtime" ];
    };
    ${pkgs.tzdata.name}.licenses = [ "BSD-3-Clause" "LicenseRef-TZ-Public-Domain" ];
    ${pkgs.iana-etc.name} = {
      # The generated registries use IANA/IETF's CC0 dedication, not the generator's MIT terms.
      licenses = [ "CC0-1.0" ];
      notice_files = [ "${./iana-licensing-terms.txt}" "${pkgs.fetchurl {
        url = "https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/CC0-1.0.txt";
        hash = "sha256-ogEPNDSH0/dhiv/lT3ifVIdgIzHAqNA/SemnxUfPBJk=";
      }}" ];
    };
    ${bashPackage.name}.components = [ "bash" ];
    ${pkgs.cacert.name}.notice_files = [ "${certdataNotice}" "${mpl2}" ];
    ${codexbarPackage.name} = {
      components = [ "codexbar" ];
      source = "${codexbarIdentity.source.url}@${codexbarIdentity.revision}";
      source_roots = [ "${codexbarSource}" ];
      notice_files = [ "${../../licenses/codexbar-MIT}" "${codexbarSource}/docs/THIRD_PARTY_LICENSES.md" ];
      embedded = [ "codexbar-static-dependencies" ];
    };
    ${installerPackage.name}.embedded = [ "rust-nixpkgs-standard-library" "musl-runtime" "installer-zstd" ];
    ${yaziPackage.name}.embedded = [
      "system-linker-runtime" "rust-nixpkgs-standard-library" "yazi-upstream-syntax-pack"
    ];
    ${eonRust.name} = {
      source = "${eonRust.src}";
      notice_files = map (notice: "${notice}") [
        ../../LICENSE ../../THIRD_PARTY_NOTICES.md ../../runtime/LICENSE ../../runtime/NOTICE
      ];
      embedded = eonStaticInputs;
    };
    ${eonPackage.name} = {
      source = "Composition of ${eonRust.name} from ${eonRust.src}";
      embedded = [ eonRust.name "eon-icon" ] ++ eonStaticInputs;
    };
    ${venusPackage.name}.embedded = [
      "system-linker-runtime" "rust-standard-library" "venus-shaders" "openai-logomark" "claude-logomark" "accesskit-chromium"
      "claude-code-mark"
      "codex-symbol"
      "opencode-mark"
      "antigravity-mark"
      "gemini-mark"
      "lucide-bot"
    ];
    ${pkgs.ncurses.name}.source = "${pkgs.ncurses.src} rebuilt with ${../../nix/ncurses-termios-speed.h}";
    ${pkgs.keyutils.name} = {
      # Only the shared library is in this closure, not the GPL utilities.
      licenses = [ "LGPL-2.1-or-later" ];
      outputs = [ (builtins.unsafeDiscardStringContext (toString pkgs.keyutils.lib)) ];
      notice_files = [ "${pkgs.runCommand "keyutils-library-copyright" { nativeBuildInputs = [ pkgs.gnutar ]; } ''
        tar -xOf ${pkgs.keyutils.src} keyutils-${pkgs.keyutils.version}/keyutils.c | sed -n '1,11p' > "$out"
      ''}" ];
    };
    ${orbitPackage.name} = {
      embedded = [ "system-linker-runtime" "rust-standard-library" "ghostty-vt" ];
      notice_files = map (notice: "${notice}") [ ../../orbit/LICENSE ../../licenses/libghostty-rs-MIT ];
      cargo_source_licenses = {
        "git+https://github.com/Uzaaft/libghostty-rs.git?rev=4466c0f45f51f964f86d1974a50502dbec7cb0de#4466c0f45f51f964f86d1974a50502dbec7cb0de" = [ "MIT" ];
      };
    };
    ${animaPackage.name}.embedded = [
      "system-linker-runtime" "rust-nixpkgs-standard-library" "accelerando-artwork"
    ];
    ${pkgs.stdenv.cc.cc.name}.licenses = [
      "GPL-3.0-or-later WITH GCC-exception-3.1"
      "GPL-3.0-or-later" # GDB helpers
      "LGPL-2.0-or-later" # libquadmath
      "MIT" # libsanitizer's compiler-rt code
    ];
    ${fzfPackage.name}.embedded = [ "go-standard-library" ];
    ${pkgs.dejavu_fonts.name} = {
      licenses = [ "Bitstream-Vera" "Arev" "AMS" ];
      source = toString pkgs.dejavu_fonts.full-ttf.src;
      source_roots = [ (toString pkgs.dejavu_fonts.full-ttf.src) ];
    };
    ${pkgs.dejavu_fonts.minimal.name} = {
      licenses = [ "Bitstream-Vera" "Arev" ];
      source = toString pkgs.dejavu_fonts.full-ttf.src;
      source_roots = [ (toString pkgs.dejavu_fonts.full-ttf.src) ];
    };
    ${pkgs.nerd-fonts.symbols-only.name} = {
      # The package's MIT notice does not cover all glyph sets in this font.
      licenses = [ "MIT" "CC-BY-4.0" "Apache-2.0" "OFL-1.1" "Unlicense" ];
      notice_files = map (source: "${pkgs.fetchurl source}")
        (builtins.fromJSON (builtins.readFile ./nerd-font-licenses.json));
      unresolved = lib.optional (pkgs.nerd-fonts.symbols-only.version != "3.4.0")
        "review the glyph license inventory for the new Symbols Only version";
    };
    helix-runtime = {
      source = "${helix}/runtime";
      licenses = [ "MPL-2.0" "MIT" "Apache-2.0" "0BSD" "Unlicense" "GPL-3.0-only" ];
      notice_files = [ "${helix}/LICENSE" "${apache2}" ];
      source_roots = [ "${helix}/runtime" "${helixRuntimeInlineNotices}" ];
      source_material = { sources = [ helixSourcePath ]; patches = [ ]; build = "${helix}/flake.nix"; };
    };
    ${anima.packages.${system}.accelerando-font.name} = {
      id = "accelerando-artwork";
      version = animaIdentity.revision;
      source = "${anima}/fonts";
      licenses = [ accelerandoGrant.license ];
      notice_files = [
        "${../../licenses/accelerando-MIT}" accelerandoGrant.record
        "${anima}/fonts/NOTICE" "${anima}/src/accelerando-glyphs.NOTICE"
      ];
      grant = accelerandoGrant;
    };
  };
  extraEntries = native.entries ++ [
    {
      id = "accesskit-chromium";
      version = "f3026222559c7f5f46d8f25cb7404de68eb149f0";
      source = "https://github.com/AccessKit/accesskit@f3026222559c7f5f46d8f25cb7404de68eb149f0 (Chromium-derived Linux accessibility code)";
      licenses = [ "BSD-3-Clause" ];
      notice_files = [ "${pkgs.runCommand "accesskit-chromium-notices" { } ''
        cat ${pkgs.fetchurl { url = "https://raw.githubusercontent.com/AccessKit/accesskit/f3026222559c7f5f46d8f25cb7404de68eb149f0/LICENSE.chromium"; hash = "sha256-hFAi4MHbGrtBprpM08S2dOwpDzNZ2dPHiuVY1MDtkwg="; }} > "$out"
        sed -n '1,9p' ${pkgs.fetchurl { url = "https://raw.githubusercontent.com/AccessKit/accesskit/f3026222559c7f5f46d8f25cb7404de68eb149f0/adapters/atspi-common/src/adapter.rs"; hash = "sha256-Lxp0mJA3kLSgCTSjItGjYziay3xmZOz2uljl3gjvOiA="; }} >> "$out"
        sed -n '1,9p' ${pkgs.fetchurl { url = "https://raw.githubusercontent.com/AccessKit/accesskit/f3026222559c7f5f46d8f25cb7404de68eb149f0/adapters/atspi-common/src/node.rs"; hash = "sha256-j6/MTxOgYcxG6gcKe04kACdIf49VB9dcRJJTyVE25M8="; }} >> "$out"
        sed -n '1,9p' ${pkgs.fetchurl { url = "https://raw.githubusercontent.com/AccessKit/accesskit/f3026222559c7f5f46d8f25cb7404de68eb149f0/accesskit_consumer/src/iterators.rs"; hash = "sha256-RU3mzLVx572DHHIbi6TrdtvvvpcEOr/HJ7kXwrzCiho="; }} >> "$out"
      ''}" ];
    }

    {
      id = "system-linker-runtime";
      inherit (pkgs.glibc) version;
      source = "${pkgs.glibc.src} and ${pkgs.stdenv.cc.cc.src}";
      licenses = [ "LGPL-2.1-or-later" "GPL-3.0-or-later WITH GCC-exception-3.1" ];
      source_roots = map toString [ pkgs.glibc.src pkgs.stdenv.cc.cc.src ];
      source_material = {
        sources = [ pkgs.glibc.src pkgs.stdenv.cc.cc.src ];
        patches = (pkgs.glibc.patches or [ ]) ++ (pkgs.stdenv.cc.cc.patches or [ ]);
        build = pkgs.writeText "eon-system-runtime-build-recipes.txt"
          "Nixpkgs ${nixpkgsSourcePath}: pkgs.glibc and pkgs.stdenv.cc.cc, as selected in the included Eon flake.nix.\n";
      };
    }
    {
      id = "rust-standard-library";
      inherit (rustToolchain) version;
      source = toString rustToolchain;
      licenses = [ "MIT" "Apache-2.0" "Apache-2.0 WITH LLVM-exception" ];
      notice_files = [ "${rustToolchain}/share/doc/rust/COPYRIGHT-library.html" ];
    }
    {
      id = "rust-nixpkgs-standard-library";
      inherit (pkgs.rustc) version;
      source = "${pkgs.rustc.src} ${pkgs.pkgsStatic.rustc.src}";
      licenses = [ "MIT" "Apache-2.0" "Apache-2.0 WITH LLVM-exception" ];
      source_roots = lib.unique (map toString [ pkgs.rustc.src pkgs.pkgsStatic.rustc.src ]);
    }
    {
      id = "musl-runtime";
      inherit (pkgs.pkgsStatic.stdenv.cc.libc) version;
      source = toString pkgs.pkgsStatic.stdenv.cc.libc.src;
      licenses = [ "MIT" ];
      source_roots = [ (toString pkgs.pkgsStatic.stdenv.cc.libc.src) ];
    }
    {
      id = "installer-zstd";
      inherit (installerZstd) version;
      source = installerZstdSource.url;
      licenses = [ "BSD-3-Clause" ];
      notice_files = [ "${installerZstdNotice}" ];
    }
    {
      id = "eon-icon";
      inherit (eonPackage) version;
      source = "${../../assets/eon.png} resized to desktop icons and converted to premultiplied RGBA";
      licenses = [ "Apache-2.0" ];
      notice_files = [ "${../../LICENSE}" ];
    }
    {
      id = "venus-shaders";
      inherit (venusPackage) version;
      source = "WGSL embedded in ${../../venus/src/render.rs} and ${../../venus/src/render/images.rs}";
      licenses = [ "Apache-2.0" ];
      notice_files = [ "${../../venus/LICENSE}" ];
    }
    {
      id = "openai-logomark";
      version = "263b2d5b7b63836c4ea30ee9709e65b7a50cbf6f";
      source = "https://github.com/openai/openai-cookbook, rasterized mask ${../../venus/assets/openai-blossom-104.alpha}";
      licenses = [ "MIT" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/openai-blossom-104.alpha;
          hash = "7bf5bf047a493ef0d76177d74a3de81783b5d1038dd13fcc6cd9b1db20edec91";
        }
      ];
    }
    {
      id = "claude-code-mark";
      version = "2.1.323";
      source = "https://marketplace.visualstudio.com/items?itemName=Anthropic.claude-code, extension/resources/clawd.svg; retrieved 2026-11-13";
      licenses = [ "LicenseRef-Anthropic-Brand-Terms" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/claude-code-clawd.svg;
          hash = "9ca6ebb33e268ff95ded9b927ceb53f22deecf4f1de9988f088437621c85154c";
        }
        {
          file = ../../venus/assets/claude-code-clawd-22.png;
          hash = "40412285c4931e7df5193473cd301f146aa23cd373e0bbfd2ea3e26e068d35e2";
        }
        {
          file = ../../venus/assets/claude-code-clawd-22.rgba;
          hash = "862167f874de6e3a2348dd2cea1cdb7add3afe10187816b748aa092e39972d64";
        }
        {
          file = ../../venus/assets/claude-code-clawd-33.png;
          hash = "8ec85689732f93a5cec80fe8d637db7f697832425a0b6f9cada289e3fbcccf41";
        }
        {
          file = ../../venus/assets/claude-code-clawd-33.rgba;
          hash = "c3d8c916c4efb305d305d2319e14c06f15e275f9f7ff1a4b47958bc72c993d51";
        }
        {
          file = ../../venus/assets/claude-code-clawd-44.png;
          hash = "2a118cb1a7e2ec3560dd4752cd4302ad17bc17b725118bd6c99db6292d09bc7c";
        }
        {
          file = ../../venus/assets/claude-code-clawd-44.rgba;
          hash = "0355088c462c27ec5fe81cdfa91b4771739e37cea08d9e617726fe9581e8cbf4";
        }
        {
          file = ../../venus/assets/claude-code-clawd-256.rgba;
          hash = "8f1969bb128bbc158d2b331ac4a428c9197a4abc921253f18c2d0d4cc7d5fb6b";
        }
      ];
    }
    {
      id = "codex-symbol";
      version = "2026-11-13";
      source = "https://zonalogo.com, Codex Logo - Colored; third-party vector, source unverified, chosen by Lucca 2026-11-13";
      licenses = [ "LicenseRef-Codex-Identification-Use" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/codex-symbol.svg;
          hash = "983fc436a025a9920ec2661d790e956f766f273a2240801e64c3d3d26feb7ad2";
        }
        {
          file = ../../venus/assets/codex-symbol-22.png;
          hash = "2184ee35861e8592e6fb2fb552f9b0ea5f851629d5dbaee02eec19f59d950ec5";
        }
        {
          file = ../../venus/assets/codex-symbol-22.rgba;
          hash = "d16e931ac2f7194bc1dedb3a0b8a8dd46f87a725b20bc2f5d1582188386afa48";
        }
        {
          file = ../../venus/assets/codex-symbol-33.png;
          hash = "6351a75207b254491927dc3ec5d6d5fb9c9a7ba10b3d883edaecca12a8c1c5cf";
        }
        {
          file = ../../venus/assets/codex-symbol-33.rgba;
          hash = "a7d821a4272dd9500a05a098b883e5d62d93d952e589c457acde0d397ed36a1c";
        }
        {
          file = ../../venus/assets/codex-symbol-44.png;
          hash = "9089fbeb8f91931754cd3fbe6d64dd5860ff4adf86e75dd45a9e6b9519008b5d";
        }
        {
          file = ../../venus/assets/codex-symbol-44.rgba;
          hash = "7be74d8cf5b1c39500867ce3e352f52d8707641e941dabd0dabeb3724606f9ea";
        }
        {
          file = ../../venus/assets/codex-symbol-256.rgba;
          hash = "307c8d69585224de9e7d9a168916300d0ca648fece87ce74a416c419716e19e6";
        }
      ];
    }
    {
      id = "opencode-mark";
      version = "2.3.0";
      source = "https://github.com/anomalyco/opencode@a6b15d33daf6ab7d35569f6d711fdb630b9f04e7";
      licenses = [ "MIT" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/opencode-mark-dark-22.png;
          hash = "79b327245fb40f98a1afe94fa6fe78b10efffeba5d8da38ca13cfe0b8f7b2fc8";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-22.rgba;
          hash = "b41492836b28f127f45ce399c8e2475c0521c13fd393b6bd70b83e4f09e1e965";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-256.rgba;
          hash = "77ded00108cd93ac0c80705643f5301292b02b5322ce8ac82ae112517f74fe42";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-33.png;
          hash = "551c9a4d42b66a4496d459f5cb58439d64d1c99ebe510bb8c748bb74de19e18e";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-33.rgba;
          hash = "5c5de890baed3c5efc0b31f3921b7c2503aded97d27d3854c62ef4bf41cdf5b1";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-44.png;
          hash = "d4e3d3e08e9f6083bea201f3817ec7091a6a2b06a4835f1ae3a6dfb7f7e535bf";
        }
        {
          file = ../../venus/assets/opencode-mark-dark-44.rgba;
          hash = "73109b07d19a0db5024f981a0b25d083af03f1c2d124d8e6e24cc91855626bab";
        }
        {
          file = ../../venus/assets/opencode-mark-dark.svg;
          hash = "502ff18efbef0432d8a702481669767c0738d744d18c45d6c3bd3b776d2d6a8e";
        }
        {
          file = ../../venus/assets/opencode-mark-light-22.png;
          hash = "ce44b030fa00100bb95ab7ba5b314b1598cce7f81c1eb1a2b6792dffb0a78e92";
        }
        {
          file = ../../venus/assets/opencode-mark-light-22.rgba;
          hash = "801a61405b0abd6195f323d771ffd7b118cf9d2f71598ade0ec79713344f3aa8";
        }
        {
          file = ../../venus/assets/opencode-mark-light-256.rgba;
          hash = "cfb7698c5bfdff524e73bc1e2c27bdc3aedc5327c79833583b32c73732427024";
        }
        {
          file = ../../venus/assets/opencode-mark-light-33.png;
          hash = "ef13165c815bf5e7af16f26f8f45f912134ddb4cc626cf4d9bbe18616f3b80c2";
        }
        {
          file = ../../venus/assets/opencode-mark-light-33.rgba;
          hash = "9091179b0a0e13939cb4940be47f51681eb4ec715bbc8ec4badfd21c05c54670";
        }
        {
          file = ../../venus/assets/opencode-mark-light-44.png;
          hash = "889caebbcbffef33f107fbd231e3ec8ebcdd9c679d8946793acaa5e02fb31041";
        }
        {
          file = ../../venus/assets/opencode-mark-light-44.rgba;
          hash = "7b87f3c78c9275cd6284d1b3f351b9762845bb4156727fcf83daae5b35c52b4f";
        }
        {
          file = ../../venus/assets/opencode-mark-light.svg;
          hash = "8fb1cffb7171c85d4c12a8219250f31a397e316b53e94ca14c171452eb3efed7";
        }
      ];
    }
    {
      id = "antigravity-mark";
      version = "2026-11-21";
      source = "https://antigravity.google/assets/image/brand/antigravity-icon__full-color.svg (from https://antigravity.google/press, Icon); retrieved 2026-11-21T15:51:02Z";
      licenses = [ "LicenseRef-Google-Brand-Terms" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/antigravity-icon-full-color.svg;
          hash = "d775965ea1f8060538c15f758a3e22487317211fcb6051333b5e023f35aa726f";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-22.png;
          hash = "81561c53e96f8dafd6063b9f5f65b478275d3a6fa87c268b75090f191ee39c5a";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-22.rgba;
          hash = "001687cd4dc87bfec63e44a756cf53068128660759690f0e3455df9e532c4419";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-33.png;
          hash = "a755f374a875a39c2b94e8632e01c7dc54a6035e10f808b117b3e2023d102878";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-33.rgba;
          hash = "7a48f70c8609ae3a8580542fd27c8057e80ffcf22b20b7df2b01637465237293";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-44.png;
          hash = "a99ebbc26d064959147b883f878054defee6521403a9d02609c82f8f0794f56d";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-44.rgba;
          hash = "f0acdd0197e59fdf3cbc7e92105ccadcc8de1718f7b580f622e857c370f8fba2";
        }
        {
          file = ../../venus/assets/antigravity-icon-full-color-256.rgba;
          hash = "d44c265dd84abc5a2929301fa734268250bc0ed62a2b6c8a7f652a5a9caac824";
        }
      ];
    }
    {
      id = "claude-logomark";
      version = codexbarIdentity.revision;
      source = "${codexbarIdentity.source.url}, rasterized mask ${../../venus/assets/claude-splat-104.alpha}";
      licenses = [ "MIT" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/claude-splat-104.alpha;
          hash = "74e02f5b35b30449941a759acb6d0872aa3d3bda0bdb78760ee0eb8893479e7f";
        }
      ];
    }
    {
      id = "gemini-mark";
      version = codexbarIdentity.revision;
      source = "${codexbarIdentity.source.url}, rasterized mask ${../../venus/assets/gemini-spark-104.alpha}";
      licenses = [ "MIT" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/gemini-spark-104.alpha;
          hash = "cc2c2e82d6cc9d4fba14a8f64b02632ae1d652d8ad839db1278650bf25404524";
        }
      ];
    }
    {
      id = "lucide-bot";
      version = "1.78.0";
      source = "https://github.com/lucide-icons/lucide@e00b2b26350fe4dfb6f5273aa366f94974430903, icons/bot.svg rasterized mask ${../../venus/assets/generic-agent-104.alpha}";
      licenses = [ "ISC" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = noticeHashes [
        {
          file = ../../venus/assets/generic-agent-104.alpha;
          hash = "e578a8aef86bd3289bed026f7e9c5e8cf1584f2ce9e641ac45ed8ea1f9f5d370";
        }
      ];
    }
    {
      id = "go-standard-library";
      inherit (pkgs.go) version;
      source = toString pkgs.go.src;
      licenses = [ "BSD-3-Clause" ];
      source_roots = [ "${pkgs.go}/share/go" ];
    }
    {
      id = "ghostty-vt";
      version = ghostty.rev;
      source = "${ghostty}";
      licenses = [ "MIT" ];
      notice_files = [ "${ghostty}/LICENSE" ghosttyNoticeCatalogue ];
      embedded = map (dependency: dependency.id) ghosttyNoticeEntries ++ [ "zig-runtime" ];
    }
    {
      id = "zig-runtime";
      inherit (zig) version;
      source = "${zig.src}";
      licenses = [ "MIT" "Apache-2.0 WITH LLVM-exception" ];
      source_roots = [ "${zig.src}" ];
    }
    {
      id = "bend-worker";
      version = bend.rev;
      source = "${bend} generates C from Orbit's Bend policy";
      licenses = [ "Apache-2.0" ];
      notice_files = [ "${bend}/LICENSE" "${../../orbit/LICENSE}" ];
    }
    {
      id = "bundle-font-configuration";
      version = "generated";
      source = "${../../flake.nix}";
      licenses = [ "Apache-2.0" ];
      outputs = [ (toString venusFontsConf) ];
      notice_files = [ "${../../LICENSE}" ];
    }
    {
      id = "bundle-yazi-configuration";
      version = "generated";
      source = "${../../flake.nix}";
      licenses = [ "Apache-2.0" ];
      outputs = [ (toString filesPopupConfig) ];
      notice_files = [ "${../../LICENSE}" ];
    }
    {
      id = "helix-runtime-data";
      version = helixIdentity.revision;
      source = "${helix}/runtime";
      licenses = [ "MPL-2.0" ];
      output_name = "runtime";
      source_material = { sources = [ helixSourcePath ]; patches = [ ]; build = "${helix}/flake.nix"; };
      notice_files = [ "${helix}/LICENSE" ];
    }
    {
      id = "helix-grammar-links";
      version = helixIdentity.revision;
      source = "${helix}/grammars.nix";
      licenses = [ "MPL-2.0" ];
      output_name = "consolidated-helix-grammars";
      source_material = { sources = [ helixSourcePath ]; patches = [ ]; build = "${helix}/grammars.nix"; };
      notice_files = [ "${helix}/LICENSE" ];
    }
    (import ./yazi-syntaxes.nix { inherit pkgs yaziPackage; })
  ] ++ ghosttyNoticeEntries ++ import ./codexbar.nix {
    inherit pkgs;
    package = codexbarPackage;
    identity = codexbarIdentity;
    source = codexbarSource;
    muslId = "musl-runtime";
  };
}
