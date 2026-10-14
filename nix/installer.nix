{ pkgs }:
let
  source = pkgs.lib.fileset.toSource {
    root = ../installer;
    fileset = pkgs.lib.fileset.unions [
      ../installer/Cargo.toml
      ../installer/Cargo.lock
      ../installer/src
      ../installer/tests
    ];
  };
in
pkgs.pkgsStatic.rustPlatform.buildRustPackage {
  pname = "eon-install";
  version = (builtins.fromTOML (builtins.readFile ../installer/Cargo.toml)).package.version;
  src = source;
  cargoLock.lockFile = ../installer/Cargo.lock;
  nativeBuildInputs = [ pkgs.binutils ];
  postInstall = ''
    headers=$(readelf -l "$out/bin/eon-install")
    if [[ "$headers" == *INTERP* ]]; then
      echo 'installer bootstrap must be static' >&2
      exit 1
    fi
    "$out/bin/eon-install" --version
  '';
  meta = {
    description = "Verified user-local installation of Eon's Linux bundle";
    license = pkgs.lib.licenses.asl20;
    mainProgram = "eon-install";
    platforms = [ "x86_64-linux" ];
  };
}
