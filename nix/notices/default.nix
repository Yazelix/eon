{ pkgs, tool, tree, manifest, packages, rustProducers, goProducers, extraEntries
, packageOverrides, grammarSources, helixSource, launcherPrograms, binaryInputs, nativeExclusions, nativeBindings
, sourceRelease, auditToolchain }:
let
  inherit (pkgs) lib;
  key = value: builtins.unsafeDiscardStringContext (toString value);
  accepted = [
    "MIT" "Apache-2.0" "Apache-2.0 WITH LLVM-exception" "BSD-2-Clause"
    "BSD-3-Clause" "BSD-1-Clause" "ISC" "Zlib" "Unicode-3.0" "BSL-1.0"
    "CC0-1.0" "Unlicense" "0BSD" "MPL-2.0" "MIT-0" "WTFPL" "CDLA-Permissive-2.0"
    "LGPL-2.0-or-later" "LGPL-2.1-only" "LGPL-2.1-or-later" "LGPL-3.0-or-later"
    "GPL-2.0-or-later" "GPL-3.0-only" "GPL-3.0-or-later" "GPL-3.0-or-later WITH GCC-exception-3.1"
    "OFL-1.1" "Bitstream-Vera" "Arev" "AMS" "Artistic-2.0" "W3C-20150513"
    "curl" "X11" "MIT-open-group" "HPND-doc" "HPND-sell-variant" "TekHVC"
    "HPND-doc-sell" "HPND" "Cronyx" "hyphen-bulgarian" "blessing" "LicenseRef-TZ-Public-Domain" "LicenseRef-Accelerando"
    "Apache-2.0 WITH Swift-exception" "OpenSSL" "ICU" "CC-BY-4.0"
    "LicenseRef-Anthropic-Brand-Terms"
    "Sublime Packages license" "OLDAP-2.8" "GPL-2.0-only WITH GCC-exception-2.0" "Unicode-DFS-2016"
  ];
  omitted = builtins.fromJSON (builtins.readFile ./omitted-grammars.json);
  grammarEvidence = builtins.fromJSON (builtins.readFile ./grammar-evidence.json);
  standardTexts = builtins.mapAttrs (_: source: pkgs.fetchurl source)
    (builtins.fromJSON (builtins.readFile ./standard-license-texts.json));
  grammarConfig = builtins.fromTOML (builtins.readFile "${helixSource}/languages.toml");
  dependencies = package: lib.filter lib.isDerivation (
    (package.buildInputs or [ ]) ++ (package.propagatedBuildInputs or [ ])
  );
  producers = builtins.genericClosure {
    startSet = map (package: { key = package.drvPath; inherit package; }) packages;
    operator = entry: map (package: { key = package.drvPath; inherit package; }) (dependencies entry.package);
  };
  sources = package:
    lib.filter (source: source != null && toString source != "") (
      [ (package.src or null) ] ++ (
        if builtins.isAttrs (package.srcs or [ ]) then builtins.attrValues package.srcs
        else package.srcs or [ ]
      )
    );
  sourceMaterial = package:
    let outputNames = package.outputs or [ "out" ];
    in {
    sources = sources package;
    patches = package.patches or [ ];
    build = pkgs.writeText "${package.name}-source-build.json"
      (lib.replaceStrings
        (map builtins.placeholder outputNames)
        (map (output: "@output:${output}@") outputNames)
        (builtins.toJSON {
        package = package.name;
        output_names = outputNames;
        inherit (pkgs.stdenv.hostPlatform) system;
        configuration = builtins.intersectAttrs (lib.genAttrs [
          "configureFlags" "configurePhase" "buildPhase" "installPhase"
          "prePatch" "postPatch" "preConfigure" "postConfigure" "preBuild" "postBuild"
          "preInstall" "postInstall" "postFixup" "makeFlags" "buildFlags" "installFlags"
          "NIX_CFLAGS_COMPILE" "NIX_LDFLAGS" "cargoBuildFeatures" "cargoBuildNoDefaultFeatures"
          "nativeBuildInputs" "buildInputs" "propagatedBuildInputs"
        ] (_: null)) package;
      }));
  };
  cargoReport = producer:
    let
      manifestPath = producer.manifest or "Cargo.toml";
      target = producer.target or "x86_64-unknown-linux-gnu";
      flags = lib.escapeShellArgs (
        lib.optional (producer.package.cargoBuildNoDefaultFeatures or false) "--no-default-features"
        ++ lib.optionals ((producer.package.cargoBuildFeatures or [ ]) != [ ])
          [ "--features" (lib.concatStringsSep " " producer.package.cargoBuildFeatures) ]
      );
      roots = lib.escapeShellArgs (lib.concatMap (root: [ "--package" root ]) producer.roots);
      auditContext = pkgs.writeText "${producer.id}-audit-context.json" (builtins.toJSON {
        producer = producer.id;
        inherit target;
        roots = producer.roots;
        default_features = !(producer.package.cargoBuildNoDefaultFeatures or false);
        features = producer.package.cargoBuildFeatures or [ ];
        build_flags = lib.toList (producer.package.cargoBuildFlags or [ ]);
        build_command = producer.package.buildPhase or "cargoBuildHook";
        toolchain = auditToolchain;
        lock_file = "Cargo.lock";
        exceptions = pkgs.writeText "${producer.id}-audit-exceptions.json"
          (builtins.toJSON (producer.auditExceptions or [ ]));
        evidence_documents."nix/notices/audit-evidence.md" = ./audit-evidence.md;
      });
    in pkgs.rustPlatform.buildRustPackage {
      pname = "${producer.id}-cargo-notices";
      version = producer.package.version or "0";
      src = producer.source or producer.package.src;
      cargoDeps = producer.package.cargoDeps;
      nativeBuildInputs = [ pkgs.cargo-about pkgs.rust-audit-info tool ];
      postPatch = lib.concatStringsSep "\n" (lib.mapAttrsToList (path: notice:
        "install -Dm444 ${notice} ${lib.escapeShellArg path}") (producer.sourceNotices or { }));
      buildPhase = ''
        mkdir -p "$out/texts"
        if cargo tree --locked --offline --target ${target} --manifest-path ${manifestPath} \
          ${flags} ${roots} --edges normal,no-proc-macro --prefix none --format '{p}' > runtime-crates.txt \
          && cargo tree --locked --offline --target ${target} --manifest-path ${manifestPath} \
            ${flags} ${roots} --edges normal,build --prefix none --format '{p} features={f}' > native-packages.txt \
          && cargo-about about generate --locked --offline --fail --target ${target} \
          --manifest-path ${manifestPath} --config ${./about.toml} --format json \
          --output-file report.json ${flags} ${lib.optionalString (builtins.length producer.roots > 1) "--workspace"} \
          && archive-notices cargo-texts report.json "$out/texts" ${producer.package}/bin runtime-crates.txt \
            ${pkgs.writeText "${producer.id}-original-crate-notices.json" (builtins.toJSON (producer.crateNotices or { }))} native-packages.txt \
            ${auditContext}; then
          :
        else
          printf '{"failed":true}\n' > report.json
        fi 2> "$out/error.txt"
        cat "$out/error.txt" >&2
      '';
      doCheck = false;
      installPhase = ''
        mkdir -p "$out/texts"
        cp report.json "$out/report.json"
      '';
      dontFixup = true;
    };
  cargoReports = lib.listToAttrs (map (producer: {
    name = key producer.package;
    value = cargoReport producer;
  }) rustProducers);
  goReport = producer:
    producer.package.overrideAttrs (old: {
    pname = "${producer.id}-go-notices";
    nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.go-licenses pkgs.jq tool ];
    buildPhase = ''
      export GOSUMDB=off GOTOOLCHAIN=local
      # Some producers pin a file proxy rather than a vendor directory.
      if [[ "$GOPROXY" == file://* ]]; then
        GOFLAGS=-mod=mod go mod vendor
      fi
      export GOPROXY=off
      # Producers such as Carapace generate imports before their real build.
      runHook preBuild
      mkdir -p report/licenses
      go version -m -json ${producer.package}/bin/${producer.binary} > report/build-info.json
      main=$(jq -er '.Path' report/build-info.json)
      setting() { jq -r --arg key "$1" '.Settings[] | select(.Key == $key).Value' report/build-info.json; }
      export CGO_ENABLED=$(setting CGO_ENABLED) GOARCH=$(setting GOARCH) GOOS=$(setting GOOS)
      tags=$(setting -tags)
      export GOFLAGS="-mod=vendor''${tags:+ -tags=$tags}"
      if go-licenses check --allowed_licenses=MIT,Apache-2.0,BSD-2-Clause,BSD-3-Clause,ISC,0BSD,Unlicense "$main" \
        && go-licenses save "$main" --save_path report/licenses --force \
        && go-licenses report "$main" > report/LICENSES.csv \
        && archive-notices check-go report/build-info.json vendor/modules.txt report/LICENSES.csv ${producer.package.go.version}; then
        printf 'passed\n' > report/status
      else
        printf 'failed\n' > report/status
        test -f report/LICENSES.csv || printf 'Go license report failed\n' > report/LICENSES.csv
      fi
    '';
    doCheck = false;
    doInstallCheck = false;
    outputs = [ "out" ];
    installPhase = ''cp -R report "$out"'';
    dontFixup = true;
  });
  goReports = lib.listToAttrs (map (producer: {
    name = key producer.package;
    value = goReport producer;
  }) goProducers);
  ownLicenses = {
    yazelix-orbit = [ "Apache-2.0" ];
    yazelix-venus = [ "Apache-2.0" ];
    helix-term = [ "MPL-2.0" ];
  };
  ownNotices = {
    yazelix-orbit = [ ../../orbit/LICENSE ];
    yazelix-venus = [ ../../venus/LICENSE ../../venus/THIRD_PARTY_NOTICES.md ];
    eon = [ ../../LICENSE ../../THIRD_PARTY_NOTICES.md ../../runtime/LICENSE ../../runtime/NOTICE ];
    eon-install = [ ../../LICENSE ];
    helix-term = [ "${helixSource}/LICENSE" ];
  };
  packageEntry = package:
    let
      name = package.pname or package.name;
      cargo = cargoReports.${key package} or null;
      go = goReports.${key package} or null;
    in {
      id = package.name;
      version = package.version or "generated";
      source = lib.concatStringsSep " " (map (source: "${source}") (sources package));
      licenses = ownLicenses.${name} or (map (license: license.spdxId or license.shortName or "UNKNOWN")
        (lib.toList (package.meta.license or [ ])));
      outputs = map (output: key package.${output}) (package.outputs or [ "out" ]);
      notice_files = map (notice: "${notice}") (ownNotices.${name} or [ ])
        ++ lib.optionals (go != null) [ "${go}/LICENSES.csv" "${go}/build-info.json" ];
      source_roots = map (source: "${source}") (sources package)
        ++ lib.optional (go != null) "${go}/licenses"
        ++ lib.optional (cargo != null) "${cargo}/texts";
      source_material = sourceMaterial package;
      cargo_reports = lib.optional (cargo != null) "${cargo}/report.json";
      checks = lib.optional (go != null) "${go}/status";
      embedded = [ "system-linker-runtime" ]
        ++ lib.optional (cargo != null) "rust-nixpkgs-standard-library"
        ++ lib.optional (go != null) "go-standard-library";
      components = lib.concatMap (producer: lib.optional (producer.package.drvPath == package.drvPath) producer.id)
        (rustProducers ++ goProducers);
      native_bindings = lib.concatMap (producer:
        lib.optionals (producer.package.drvPath == package.drvPath) (nativeBindings.${producer.id} or [ ])) rustProducers;
      upstream_distribution = lib.findFirst (distribution: distribution != null) null
        (map (producer: if producer.package.drvPath == package.drvPath && (producer.nixpkgs or false)
          then "the pinned Nixpkgs build of ${package.name}" else null) rustProducers);
    } // (packageOverrides.${package.name} or { });
  grammarEntry = grammar:
    let
      evidence = grammarEvidence.${grammar.name} or { revision = ""; licenses = [ ]; evidence = [ ]; };
      source = "${grammarSources}/${grammar.name}";
    in {
      id = "helix-grammar-${grammar.name}";
      version = grammar.source.rev;
      source = "${grammar.source.git}@${grammar.source.rev}";
      grammar = grammar.name;
      output_name = "helix-tree-sitter-${grammar.name}-${grammar.source.rev}";
      licenses = evidence.licenses;
      source_material = {
        sources = [ source ];
        patches = [ ];
        build = "${helixSource}/grammars.nix";
      };
      notice_files = map (file: "${source}/${file.path}") evidence.evidence
        ++ map (license: "${standardTexts.${license}}") (evidence.terms or [ ]);
      notice_hashes = lib.listToAttrs (map (file: {
        name = key "${source}/${file.path}";
        value = file.sha256;
      }) evidence.evidence);
      embedded = [ "system-linker-runtime" ];
      unresolved = lib.optional (evidence.revision != grammar.source.rev) "grammar source revision needs license review";
    };
  input = pkgs.writeText "eon-archive-notice-input.json" (builtins.toJSON {
    schema = 1;
    inherit accepted;
    source_release = sourceRelease;
    native_exclusions = nativeExclusions;
    omitted_grammars = omitted;
    # Ratconfig has a composition pin but no shipped binary or linked Cargo package.
    required_components = with manifest.composition;
      [ "eon" "installer" ] ++ services ++ clients ++ tools ++ lib.remove "ratconfig" libraries;
    binary_inputs = lib.mapAttrs' (program: dependencies: {
      name = key (lib.removePrefix "/" program);
      value = dependencies;
    }) (binaryInputs // lib.listToAttrs (map (program: {
      name = key program;
      value = [ "archive-launcher" "rust-nixpkgs-standard-library" "musl-runtime" ];
    }) launcherPrograms));
    entries = map (entry: packageEntry entry.package) producers
      ++ map grammarEntry (builtins.filter (grammar: grammar.source ? git) grammarConfig.grammar)
      ++ extraEntries;
  });
  notices = pkgs.runCommand "eon-archive-third-party-notices" {
    nativeBuildInputs = [ tool pkgs.rust-audit-info pkgs.binutils pkgs.gnutar pkgs.xz pkgs.bzip2 pkgs.lzip pkgs.zstd ];
  } ''
    archive-notices generate ${input} ${tree} "$out"
  '';
  check = pkgs.runCommand "eon-archive-notices-check" {
    nativeBuildInputs = [ tool ];
  } ''
    archive-notices check ${notices}
    touch "$out"
  '';
in
{
  inherit input notices check;
}
