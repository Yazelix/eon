let
  eon = builtins.getFlake ("path:" + toString ../.);
  pkgs = import eon.inputs.nixpkgs { system = "x86_64-linux"; };
  producer = eon.inputs.runtime;
  inherit (eon.inputs) venus;
  candidate = builtins.fromJSON (builtins.readFile ../components/eon-alpha-v3.json);
  component = id: builtins.head (builtins.filter (x: x.id == id) candidate.components);
  codecIdentity = component "eon-workspace-protocol";
  runtimeIdentity = component "eon-runtime";
  venusIdentity = component "venus";
  orbitProtocolDependency = (builtins.fromTOML
    (builtins.readFile "${producer}/crates/eon-runtime/Cargo.toml")).dependencies.orbit-protocol;
  interface = builtins.head codecIdentity.interfaces;
  revision = codecIdentity.revision;
  proofSource = builtins.fetchGit {
    url = codecIdentity.source.url;
    rev = interface.proof;
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
    for name in orbit-content orbit-build-input; do
      cp -R ${eon.inputs.orbit} "$out/$name"
      chmod -R u+w "$out/$name"
    done
    printf '\n// codec drift\n' >> "$out/orbit-content/crates/protocol/src/lib.rs"
    mkdir -p "$out/orbit-build-input/.cargo"
    printf '[build]\nrustflags = ["--cfg", "fixture"]\n' > "$out/orbit-build-input/.cargo/config.toml"
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
    # Exercise the actual production translation with Cargo's two Git codec
    # identities, rather than only checking the package-selection predicate.
    mkdir -p "$out/composed/nix" "$out/composed/assets"
    for name in Cargo.toml flake.nix flake.lock LICENSE crates components licenses; do
      cp -R ${eon.outPath}/"$name" "$out/composed/$name"
    done
    cp ${eon.packages.x86_64-linux.default.src}/Cargo.lock "$out/composed/Cargo.lock"
    cp ${./workspace-package.nix} "$out/composed/nix/workspace-package.nix"
    cp ${../assets/eon.png} "$out/composed/assets/eon.png"
    chmod -R u+w "$out/composed"
    sed -i '/^      "id": "eon-runtime",$/,/"target":/s/${runtimeIdentity.revision}/${branchRevision}/' "$out/composed/components/eon-alpha-v3.json"
    substituteInPlace "$out/composed/crates/eon/Cargo.toml" --replace-fail \
      'eon-runtime = { git = "${runtimeIdentity.source.url}", rev = "${runtimeIdentity.revision}" }' \
      'eon-runtime = { git = "${runtimeIdentity.source.url}", rev = "${branchRevision}" }'
    # Restore controlled Git identities from the single-package production lock.
    sed -i '
      /name = "eon-runtime"/,/^\[\[package\]\]$/ { /^version =/a source = "git+${runtimeIdentity.source.url}?rev=${branchRevision}#${branchRevision}"
      }
      /name = "eon-workspace-protocol"/,/^\[\[package\]\]$/ { /^version =/a source = "git+${codecIdentity.source.url}?rev=${revision}#${revision}"
      }
      /name = "orbit-protocol"/,/^\[\[package\]\]$/ { /^version =/a source = "git+${orbitProtocolDependency.git}?rev=${orbitProtocolDependency.rev}#${orbitProtocolDependency.rev}"
      }
    ' "$out/composed/Cargo.lock"
    sed -i '/name = "eon"/,/^\[\[package\]\]$/s|"eon-workspace-protocol"|"eon-workspace-protocol ${codecIdentity.version} (git+${codecIdentity.source.url}?rev=${revision})"|' "$out/composed/Cargo.lock"
    sed -i '/name = "eon-runtime"/,/^\[\[package\]\]$/s|"eon-workspace-protocol"|"eon-workspace-protocol ${codecIdentity.version} (git+${runtimeIdentity.source.url}?rev=${branchRevision})"|' "$out/composed/Cargo.lock"
    printf '\n[[package]]\nname = "eon-workspace-protocol"\nversion = "${codecIdentity.version}"\nsource = "git+${runtimeIdentity.source.url}?rev=${branchRevision}#${branchRevision}"\n' >> "$out/composed/Cargo.lock"
  '';
  inputs = {
    inherit runtimeIdentity codecIdentity venusIdentity;
    runtimeSource = producer;
    codecSource = eon.inputs.workspaceProtocol;
    codecProofSource = proofSource;
    venusSource = venus;
  };
  gate = import ./workspace-package.nix;
  source = name: { outPath = "${fixtures}/${name}"; rev = branchRevision; };
  accepted = overrides: (builtins.tryEval (gate (inputs // overrides))).success;
  rejected = overrides: !(accepted overrides);
  runtimeOnly = runtimeDrift "runtime-only";
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
  composed = (import "${fixtures}/composed/flake.nix").outputs
    (eon.inputs // { self = eon; runtime = source "runtime-only"; });
  rejectedOrbit = name: !(builtins.tryEval ((import ../flake.nix).outputs
    (eon.inputs // { self = eon; orbit = eon.inputs.orbit // {
      outPath = "${fixtures}/${name}";
    }; })).packages.x86_64-linux.default.src.drvPath).success;
  normalizedSource = composed.packages.x86_64-linux.default.src;
  normalizedCodec = builtins.filter (x: x.name == codecIdentity.id)
    (builtins.fromTOML (builtins.readFile "${normalizedSource}/Cargo.lock")).package;
  validator = pkgs.rustPlatform.buildRustPackage {
    pname = "eon-manifest-check";
    version = (builtins.fromTOML (builtins.readFile ../crates/eon-manifest/Cargo.toml)).package.version;
    src = normalizedSource;
    cargoLock.lockFile = "${normalizedSource}/Cargo.lock";
    cargoBuildFlags = [ "--package" "eon-manifest" ];
    doCheck = false;
  };
  json = name: value: pkgs.writeText name (builtins.toJSON value);
in
assert accepted {};
assert accepted runtimeOnly;
assert builtins.all rejectedOrbit [ "orbit-content" "orbit-build-input" ];
assert builtins.all (name: rejected (runtimeDrift name)) [ "content" "manifest" "build-input" "lock" "ambiguous" "target-dependency" "dev-only" ];
assert builtins.all (name: rejected (venusDrift name)) [ "venus-lock" "venus-manifest" "venus-dev-only" "venus-patch" "venus-replace" "venus-build-input" ];
assert rejected { codecProofSource = proofSource // { rev = branchRevision; }; };
assert rejected { codecSource = builtins.removeAttrs eon.inputs.workspaceProtocol [ "rev" ]; };
assert rejected { codecProofSource = proofSource // { dirtyRev = "${interface.proof}-dirty"; }; };
assert rejected { runtimeSource = producer // { rev = branchRevision; }; };
assert rejected { codecSource = { outPath = "${fixtures}/content"; rev = revision; }; };
assert rejected { codecProofSource = { outPath = "${fixtures}/content"; rev = interface.proof; }; };
assert rejected { codecIdentity = codecIdentity // { interfaces = [ interface interface ]; }; };
assert normalizedCodec == [ { name = codecIdentity.id; version = codecIdentity.version; } ];
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
  ln -s ${normalizedSource} "$out/normalized-source"
''
