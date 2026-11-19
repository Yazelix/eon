# The unsafe policy in docs/UNSAFE.md: the checker's own tests, then the checker over every
# Rust file, manifest, Nix file, workflow and Cargo configuration in the repository.
{ pkgs, rustToolchain }:
let
  inherit (pkgs) lib;
  source = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../docs/UNSAFE.md
      (lib.fileset.fileFilter (
        file:
        lib.any file.hasExt [ "rs" "nix" "yml" "yaml" ]
        || lib.elem file.name [ "Cargo.toml" "config" "config.toml" ]
      ) ../.)
    ];
  };
in
pkgs.runCommand "eon-unsafe-policy" { nativeBuildInputs = [ rustToolchain ]; } ''
  rustc --edition=2024 -D warnings --test ${./unsafe-policy-check.rs} -o unit
  ./unit
  rustc --edition=2024 -D warnings ${./unsafe-policy-check.rs} -o check
  ./check ${source}
  touch "$out"
''
