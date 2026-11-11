{ pkgs, system, package, helixPackage }:
let
  inherit (pkgs) lib;
  closure = pkgs.closureInfo { rootPaths = [ package ]; };
  # Compare store paths without adding them as inputs to the check.
  exactPaths = map (path: lib.escapeRegex (builtins.unsafeDiscardStringContext (toString path))) [
    helixPackage
    helixPackage.HELIX_DEFAULT_RUNTIME
  ];
  helixNames = "${lib.escapeRegex builtins.storeDir}/[0-9a-z]{32}-(helix|yazelix-helix)(-.*)?";
  forbidden = "^(${lib.concatStringsSep "|" (exactPaths ++ [ helixNames ])})$";
in
pkgs.runCommand "eon-no-helix-closure-check" { nativeBuildInputs = [ pkgs.jq ]; } ''
  if grep -E ${lib.escapeShellArg forbidden} ${closure}/store-paths; then
    echo 'Helix store paths found in the no-helix closure' >&2
    exit 1
  elif test $? != 1; then
    exit 1
  fi

  for dir in ${package}/bin ${package}/libexec/eon/bin; do
    for name in hx eon-hx; do
      if test -e "$dir/$name" || test -L "$dir/$name"; then
        echo "Unexpected Helix command: $dir/$name" >&2
        exit 1
      fi
    done
  done
  test ! -e ${package}/share/licenses/eon/helix
  PATH=${package}/libexec/eon/bin ${pkgs.bash}/bin/bash -c '! command -v hx && ! command -v eon-hx'

  versions=$(EON_HX=/inherited/hx ${package}/bin/eon versions)
  if grep -E '^helix[[:space:]]' <<< "$versions"; then
    echo 'eon versions reported Helix in the no-helix package' >&2
    exit 1
  elif test $? != 1; then
    exit 1
  fi

  jq -e --arg system ${system} '
    .platform == $system and .attribute == ("packages." + $system + ".no-helix")
  ' ${package}/share/eon-source-receipt.json
  touch "$out"
''
