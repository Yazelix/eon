{ pkgs, manifest, noticeTool, bundleRelink, helix, yazi, ghostty, ghosttyDeps, bend, anima, nixpkgsSource
, zig, rustToolchain, system, withoutAccelerando, githubSource, codexbarIdentity
, animaIdentity, helixIdentity, venusFonts, venusBuildInputs, venusRuntimeLibraries
, venusFontsConf, filesPopupConfig, eonRust, eonPackage, orbitPackage, venusPackage, installerPackage
, animaPackage, nushellPackage, bashPackage, starshipPackage, zoxidePackage, fzfPackage
, atuinPackage, carapacePackage, helixPackage, yaziPackage, lazygitPackage, codexbarPackage }:
let
  inherit (pkgs) lib;
  launcher = bundleRelink.launcher;
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
  accelerandoGrantPath = "${anima}/fonts/redistribution-grant.json";
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
  accelerandoGrant = if !builtins.pathExists accelerandoGrantPath then null else
    let
      grant = builtins.fromJSON (builtins.readFile accelerandoGrantPath);
      asset = relative: hash: {
        name = builtins.unsafeDiscardStringContext "${anima}/${relative}";
        value = hash;
      };
    in {
      inherit (grant) rights_holder public_binary_redistribution modified_regular_font
        modified_bold_font bitmap_glyphs recipients_rights attribution revocability;
      record = accelerandoGrantPath;
      sha256 = builtins.hashFile "sha256" accelerandoGrantPath;
      assets = lib.listToAttrs [
        (asset "fonts/accelerando-terminal.ttf" grant.regular_sha256)
        (asset "fonts/accelerando-terminal-bold.ttf" grant.bold_sha256)
        (asset "src/accelerando-glyphs.bin" grant.bitmaps_sha256)
      ];
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
      source = "${eonRust.src}" + lib.optionalString withoutAccelerando
        " compiled with EON_WITHOUT_ACCELERANDO=1";
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
        "git+https://github.com/Uzaaft/libghostty-rs.git?rev=9b598d6528c08c013407052767bb2e885ba869ba#9b598d6528c08c013407052767bb2e885ba869ba" = [ "MIT" ];
      };
    };
    ${animaPackage.name} = {
      source = "${animaPackage.src}" + lib.optionalString withoutAccelerando
        " patched with ${./anima-without-accelerando.patch}";
      notice_files = lib.optional withoutAccelerando "${pkgs.writeText "anima-eon-modifications.NOTICE" ''
        Eon modifies Anima ${animaIdentity.revision} for the archive without Accelerando.
        Modified files: src/matrix.rs, src/random.rs, src/screen_runner.rs.
        These changes omit Accelerando selection and embedded artwork; Game of Life remains.
        Original code remains under Apache-2.0; its license accompanies this notice.
      ''}";
      embedded = [ "system-linker-runtime" "rust-nixpkgs-standard-library" ]
        ++ lib.optional (!withoutAccelerando) "accelerando-artwork";
      forbidden_embedded_files = lib.optional withoutAccelerando "${anima}/src/accelerando-glyphs.bin";
    };
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
  } // lib.optionalAttrs (!withoutAccelerando) {
    ${anima.packages.${system}.accelerando-font.name} = {
      id = "accelerando-artwork";
      version = animaIdentity.revision;
      source = "${anima}/fonts";
      licenses = [ "LicenseRef-Accelerando" ];
      notice_files = [ "${anima}/fonts/NOTICE" "${anima}/src/accelerando-glyphs.NOTICE" ]
        ++ lib.optional (accelerandoGrant != null) accelerandoGrantPath;
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
      notice_hashes = {
        ${builtins.unsafeDiscardStringContext "${../../venus/assets/openai-blossom-104.alpha}"} =
          "7bf5bf047a493ef0d76177d74a3de81783b5d1038dd13fcc6cd9b1db20edec91";
      };
    }
    {
      id = "claude-code-mark";
      version = "2.1.323";
      source = "https://marketplace.visualstudio.com/items?itemName=Anthropic.claude-code, extension/resources/clawd.svg; retrieved 2026-11-13";
      licenses = [ "LicenseRef-Anthropic-Brand-Terms" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = {
        ${builtins.unsafeDiscardStringContext "${../../venus/assets/claude-code-clawd.svg}"} =
          "9ca6ebb33e268ff95ded9b927ceb53f22deecf4f1de9988f088437621c85154c";
        ${builtins.unsafeDiscardStringContext "${../../venus/assets/claude-code-clawd-256.rgba}"} =
          "8f1969bb128bbc158d2b331ac4a428c9197a4abc921253f18c2d0d4cc7d5fb6b";
      };
    }
    {
      id = "claude-logomark";
      version = codexbarIdentity.revision;
      source = "${codexbarIdentity.source.url}, rasterized mask ${../../venus/assets/claude-splat-104.alpha}";
      licenses = [ "MIT" ];
      notice_files = [ "${../../venus/THIRD_PARTY_NOTICES.md}" ];
      notice_hashes = {
        ${builtins.unsafeDiscardStringContext "${../../venus/assets/claude-splat-104.alpha}"} =
          "74e02f5b35b30449941a759acb6d0872aa3d3bda0bdb78760ee0eb8893479e7f";
      };
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
