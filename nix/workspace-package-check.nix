let
  eon = builtins.getFlake ("path:" + toString ../.);
  pkgs = import eon.inputs.nixpkgs { system = "x86_64-linux"; };
  revision = "b8f18b4374ca0818a64d9cac6a3fcd02d9f2f2aa";
  producer = builtins.fetchGit {
    url = "https://github.com/Yazelix/eon-runtime.git";
    rev = revision;
  };
  venus = builtins.fetchGit {
    url = "https://github.com/Yazelix/eon-desktop.git";
    rev = "bbf4cf41b289f441683eb7a3f225a26a4d3edacc";
  };
  canonical = builtins.fromJSON (builtins.readFile ../components/eon-alpha-v3.json);
  interface = { id = "EONW"; version = 7; proof = revision; };
  requirement = {
    component = "eon-workspace-protocol";
    inherit revision;
    interfaces = [ interface ];
  };
  package = name: {
    id = name;
    project = "Eon Runtime";
    version = (builtins.fromTOML (builtins.readFile "${producer}/crates/${name}/Cargo.toml")).package.version;
    inherit revision;
    target = canonical.product.target;
    source = { kind = "git"; url = "https://github.com/Yazelix/eon-runtime.git"; };
    artifacts = [ { id = name; kind = "cargo-package"; } ];
    contracts = [];
    interfaces = [];
    requires = [];
  };
  codecIdentity = (package "eon-workspace-protocol") // { interfaces = [ interface ]; };
  runtimeIdentity = (package "eon-runtime") // { requires = [ requirement ]; };
  venusIdentity = (builtins.head (builtins.filter (x: x.id == "venus") canonical.components)) // {
    revision = venus.rev;
    requires = (builtins.head (builtins.filter (x: x.id == "venus") canonical.components)).requires ++ [ requirement ];
  };
  candidate = canonical // {
    composition = canonical.composition // {
      libraries = canonical.composition.libraries ++ [ runtimeIdentity.id codecIdentity.id ];
    };
    components = map (x: if x.id == "venus" then venusIdentity else x) canonical.components
      ++ [ runtimeIdentity codecIdentity ];
  };
  # Synthetic revisions exercise selection branches; these are not accepted sources.
  branchRevision = "1111111111111111111111111111111111111111";
  fixtures = pkgs.runCommand "eon-workspace-package-fixtures" {} ''
    mkdir -p "$out"
    for name in runtime-only content manifest build-input lock ambiguous target-dependency dev-only; do
      cp -R ${producer} "$out/$name"
      chmod -R u+w "$out/$name"
    done
    printf '\n// runtime-only fixture\n' >> "$out/runtime-only/crates/eon-runtime/src/lib.rs"
    substituteInPlace "$out/runtime-only/Cargo.lock" --replace-fail 'name = "itoa"' 'name = "unrelated-fixture"'
    printf '\n[workspace.package]\nversion = "9.9.9"\n' >> "$out/runtime-only/Cargo.toml"
    printf '\n// codec drift\n' >> "$out/content/crates/eon-workspace-protocol/src/v7.rs"
    printf '\n[features]\nfixture = []\n' >> "$out/manifest/crates/eon-workspace-protocol/Cargo.toml"
    mkdir -p "$out/build-input/.cargo"
    printf '[build]\nrustflags = ["--cfg", "fixture"]\n' > "$out/build-input/.cargo/config.toml"
    substituteInPlace "$out/lock/Cargo.lock" --replace-fail 'name = "eon-workspace-protocol"' 'name = "wrong-codec"'
    printf '\n[[package]]\nname = "eon-workspace-protocol"\nversion = "0.1.0"\n' >> "$out/ambiguous/Cargo.lock"
    printf '\n[target.\x27cfg(unix)\x27.dependencies]\neon-workspace-protocol = { path = "../other-codec" }\n' >> "$out/target-dependency/crates/eon-runtime/Cargo.toml"
    substituteInPlace "$out/dev-only/crates/eon-runtime/Cargo.toml" --replace-fail 'eon-workspace-protocol = { path = "../eon-workspace-protocol" }' ""
    printf '\n[dev-dependencies]\neon-workspace-protocol = { path = "../eon-workspace-protocol" }\n' >> "$out/dev-only/crates/eon-runtime/Cargo.toml"
    for name in venus-lock venus-manifest venus-dev-only venus-patch venus-replace venus-build-input; do
      mkdir "$out/$name"
      cp ${venus}/Cargo.{toml,lock} "$out/$name/"
      chmod -R u+w "$out/$name"
    done
    substituteInPlace "$out/venus-lock/Cargo.lock" --replace-fail '#${revision}"' '#${branchRevision}"'
    substituteInPlace "$out/venus-manifest/Cargo.toml" --replace-fail '${revision}' '${branchRevision}'
    substituteInPlace "$out/venus-dev-only/Cargo.toml" --replace-fail 'eon-workspace-protocol = { git = "https://github.com/Yazelix/eon-runtime.git", rev = "${revision}" }' ""
    printf '\n[dev-dependencies]\neon-workspace-protocol = { git = "https://github.com/Yazelix/eon-runtime.git", rev = "${revision}" }\n' >> "$out/venus-dev-only/Cargo.toml"
    printf '\n[patch."https://github.com/Yazelix/eon-runtime.git"]\ncodec = { package = "eon-workspace-protocol", path = "other-codec" }\n' >> "$out/venus-patch/Cargo.toml"
    printf '\n[replace]\n"eon-workspace-protocol:0.1.0" = { path = "other-codec" }\n' >> "$out/venus-replace/Cargo.toml"
    mkdir "$out/venus-build-input/.cargo"
    printf '[build]\nrustflags = ["--cfg", "fixture"]\n' > "$out/venus-build-input/.cargo/config.toml"
  '';
  inputs = {
    inherit runtimeIdentity codecIdentity venusIdentity;
    runtimeSource = producer;
    codecSource = producer;
    codecProofSource = producer;
    venusSource = venus;
  };
  gate = import ./workspace-package.nix;
  source = name: { outPath = "${fixtures}/${name}"; rev = branchRevision; };
  accepted = overrides: (builtins.tryEval (gate (inputs // overrides))).success;
  rejected = overrides: !(accepted overrides);
  runtimeOnly = {
    runtimeIdentity = runtimeIdentity // { revision = branchRevision; };
    runtimeSource = source "runtime-only";
  };
  runtimeDrift = name: {
    runtimeIdentity = runtimeIdentity // { revision = branchRevision; };
    runtimeSource = source name;
  };
  venusDrift = name: {
    venusIdentity = venusIdentity // { revision = branchRevision; };
    venusSource = source name;
  };
  distinctGraph = candidate // {
    components = map (x: if x.id == runtimeIdentity.id then runtimeOnly.runtimeIdentity else x) candidate.components;
  };
  wrongRevision = candidate // {
    components = map (x: if x.id == "venus" then x // {
      requires = map (r: if r.component == codecIdentity.id then r // { revision = branchRevision; } else r) x.requires;
    } else x) candidate.components;
  };
  wrongProof = candidate // {
    components = map (x: if x.id == codecIdentity.id then x // {
      interfaces = [ (interface // { proof = branchRevision; }) ];
    } else x) candidate.components;
  };
  validator = pkgs.rustPlatform.buildRustPackage {
    pname = "eon-manifest-check";
    version = (builtins.fromTOML (builtins.readFile ../crates/eon-manifest/Cargo.toml)).package.version;
    src = eon.packages.x86_64-linux.default.src;
    cargoLock.lockFile = "${eon.packages.x86_64-linux.default.src}/Cargo.lock";
    cargoBuildFlags = [ "--package" "eon-manifest" ];
    doCheck = false;
  };
  json = name: value: pkgs.writeText name (builtins.toJSON value);
in
assert accepted {};
assert accepted runtimeOnly;
assert builtins.all (name: rejected (runtimeDrift name)) [ "content" "manifest" "build-input" "lock" "ambiguous" "target-dependency" "dev-only" ];
assert builtins.all (name: rejected (venusDrift name)) [ "venus-lock" "venus-manifest" "venus-dev-only" "venus-patch" "venus-replace" "venus-build-input" ];
assert rejected { codecProofSource = producer // { rev = branchRevision; }; };
assert rejected { codecSource = builtins.removeAttrs producer [ "rev" ]; };
assert rejected { codecProofSource = producer // { dirtyRev = "${revision}-dirty"; }; };
assert rejected { runtimeSource = producer // { rev = branchRevision; }; };
assert rejected { codecSource = { outPath = "${fixtures}/content"; rev = revision; }; };
assert rejected { codecProofSource = { outPath = "${fixtures}/content"; rev = revision; }; };
assert rejected { codecIdentity = codecIdentity // { interfaces = [ interface interface ]; }; };
pkgs.runCommand "eon-workspace-package-check" {} ''
  ${validator}/bin/eon-manifest ${json "accepted-packages.json" candidate}
  ${validator}/bin/eon-manifest ${json "distinct-packages.json" distinctGraph}
  if ${validator}/bin/eon-manifest ${json "wrong-codec-revision.json" wrongRevision}; then
    echo 'validator accepted a mismatched codec requirement' >&2; exit 1
  fi
  if ${validator}/bin/eon-manifest ${json "wrong-codec-proof.json" wrongProof}; then
    echo 'validator accepted an unaccepted codec interface proof' >&2; exit 1
  fi
  mkdir "$out"
  ln -s ${gate inputs} "$out/codec"
''
