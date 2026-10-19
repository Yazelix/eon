# The eon closure relinked to run on the host glibc without a namespace: every
# program interpreter becomes the x86-64 psABI path, store RUNPATH entries
# become $ORIGIN-relative, and absolute store symlinks become relative. The host
# supplies glibc and the Vulkan driver, so glibc, Mesa and what only they
# reference stay out of the tree. The gates hold every ELF to the glibc floor
# and resolve each NEEDED library inside the tree or on the declared host
# libraries.
{ pkgs, eonPackage }:
let
  relink = pkgs.rustPlatform.buildRustPackage {
    pname = "eon-relink";
    version = (builtins.fromTOML (builtins.readFile ../relink/Cargo.toml)).package.version;
    src = pkgs.lib.fileset.toSource {
      root = ../relink;
      fileset = pkgs.lib.fileset.unions [
        ../relink/Cargo.toml
        ../relink/Cargo.lock
        ../relink/src
      ];
    };
    cargoLock.lockFile = ../relink/Cargo.lock;
    # The tests shape and inspect a real ELF.
    nativeBuildInputs = [ pkgs.binutils pkgs.patchelf ];
  };
  closure = pkgs.closureInfo { rootPaths = [ eonPackage ]; };
in
pkgs.runCommand "eon-bundle-relink" {
  nativeBuildInputs = [ relink pkgs.binutils pkgs.patchelf ];
} ''
  hostLibraries=(--host-library libgcc_s.so.1 --host-library libwayland-client.so.0)
  for library in ${pkgs.glibc}/lib/*.so.*; do
    hostLibraries+=(--host-library "''${library##*/}")
  done
  eon-relink --registration ${closure}/registration --root ${eonPackage} \
    --exclude ${pkgs.glibc} --exclude ${pkgs.mesa} --glibc-floor 2.42 \
    "''${hostLibraries[@]}" "$out"
''
