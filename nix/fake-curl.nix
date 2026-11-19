# eon-bench's fake network for the Nix-free update journey: a static curl
# stand-in that runs inside the bench's jail, where no Nix-linked program can.
{ pkgs }:
let
  lib = pkgs.lib;
  root = ../eon/tools/fake-curl;
in
pkgs.pkgsStatic.rustPlatform.buildRustPackage {
  pname = "eon-fake-curl";
  version = (builtins.fromTOML (builtins.readFile (root + "/Cargo.toml"))).package.version;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/src")
    ];
  };
  cargoLock.lockFile = root + "/Cargo.lock";
  nativeBuildInputs = [ pkgs.binutils ];
  postInstall = ''
    headers=$(readelf -l "$out/bin/eon-fake-curl")
    if [[ "$headers" == *INTERP* ]]; then
      echo "the fake curl must be static" >&2
      exit 1
    fi
  '';
}
