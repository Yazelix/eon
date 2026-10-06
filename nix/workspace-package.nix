# Private source-preparation gate. Call only after eon-manifest validates the
# selected graph; supply immutable fetched sources and the accepted EONW proof.
{ runtimeIdentity, codecIdentity, venusIdentity
, runtimeSource, codecSource, codecProofSource, venusSource
}:
let
  codecName = "eon-workspace-protocol";
  runtimeName = "eon-runtime";
  readToml = path:
    assert builtins.pathExists path;
    builtins.fromTOML (builtins.readFile path);
  manifest = source: name: readToml "${source}/crates/${name}/Cargo.toml";
  codec = source: builtins.path {
    path = "${source}/crates/${codecName}";
    name = codecName;
  };
  sameRevision = source: identity:
    !(source ? dirtyRev) && (source.rev or null) == identity.revision;
  interfaces = builtins.filter (x: x.id == "EONW") codecIdentity.interfaces;
  proof = builtins.head interfaces;
  selected = codec codecSource;
  codecManifest = manifest codecSource codecName;
  runtimeManifest = manifest runtimeSource runtimeName;
  venusManifest = readToml "${venusSource}/Cargo.toml";
  lockPackage = source:
    let matches = builtins.filter (x: x.name == codecName)
      (readToml "${source}/Cargo.lock").package;
    in assert builtins.length matches == 1; builtins.head matches;
  codecDependency = cargo:
    let references = builtins.concatMap (table:
      builtins.concatMap (kind:
        let deps = table.${kind} or {};
        in builtins.filter (name: name == codecName || (builtins.isAttrs deps.${name}
          && (deps.${name}.package or null) == codecName)) (builtins.attrNames deps))
        [ "dependencies" "build-dependencies" "dev-dependencies" ])
      ([ cargo ] ++ builtins.attrValues (cargo.target or {})
        ++ map (dependencies: { inherit dependencies; }) (builtins.attrValues (cargo.patch or {})));
    in assert builtins.length references == 1;
      cargo.dependencies.${codecName} or null;
  # No inherited fields, dependencies or out-of-package build inputs are proved
  # for this codec. A change in that shape requires a renewed bounded proof.
  noCargoConfig = source: directories:
    builtins.all (dir: builtins.all (name:
      !(builtins.pathExists "${source}/${dir}${name}"))
      [ ".cargo/config" ".cargo/config.toml" ]) directories;
  boundedProducer = source:
    let root = readToml "${source}/Cargo.toml";
    in builtins.attrNames root == [ "workspace" ]
      && root.workspace.resolver == "3"
      && noCargoConfig source
        [ "" "crates/" "crates/${codecName}/" "crates/${runtimeName}/" ];
  packageFields = [ "edition" "license" "name" "publish" "rust-version" "version" ];
  gitSource = "git+${codecIdentity.source.url}?rev=${codecIdentity.revision}#${codecIdentity.revision}";
in
assert sameRevision runtimeSource runtimeIdentity;
assert sameRevision codecSource codecIdentity;
assert sameRevision venusSource venusIdentity;
assert builtins.length interfaces == 1 && proof.version == 7;
assert sameRevision codecProofSource { revision = proof.proof; };
assert codecIdentity.id == codecName && runtimeIdentity.id == runtimeName;
assert codecIdentity.source.kind == "git";
assert builtins.all boundedProducer [ runtimeSource codecSource codecProofSource ];
assert noCargoConfig venusSource [ "" ];
assert !(venusManifest ? replace);
assert builtins.attrNames codecManifest == [ "package" ];
assert builtins.attrNames codecManifest.package == packageFields;
assert builtins.all (name: !(builtins.isAttrs codecManifest.package.${name})) packageFields;
assert !(builtins.pathExists "${selected}/build.rs");
assert codecManifest.package.name == codecName;
assert codecManifest.package.version == codecIdentity.version;
assert selected == codec codecProofSource && selected == codec runtimeSource;
assert runtimeManifest.package.name == runtimeName;
assert runtimeManifest.package.version == runtimeIdentity.version;
assert codecDependency runtimeManifest == { path = "../${codecName}"; };
assert lockPackage runtimeSource == { name = codecName; version = codecIdentity.version; };
assert codecDependency venusManifest == {
  git = codecIdentity.source.url;
  rev = codecIdentity.revision;
};
assert lockPackage venusSource == {
  name = codecName;
  version = codecIdentity.version;
  source = gitSource;
};
selected
