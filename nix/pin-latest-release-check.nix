# .github/scripts/pin-latest-release.sh against real Git tags and a stub cachix
# (nix/pin-latest-release-check.rs).
{ pkgs, rustToolchain }:
pkgs.runCommand "eon-pin-latest-release" { nativeBuildInputs = [ rustToolchain pkgs.git ]; } ''
  rustc --edition=2024 -D warnings ${./pin-latest-release-check.rs} -o check
  ./check ${../.github/scripts/pin-latest-release.sh}
  touch "$out"
''
