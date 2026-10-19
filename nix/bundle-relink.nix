# The eon closure relinked to run on the host glibc without a namespace: every
# program interpreter becomes the x86-64 psABI path, store RUNPATH entries
# become $ORIGIN-relative, and absolute store symlinks become relative. The host
# supplies glibc and the Vulkan driver, so glibc, Mesa and what only they
# reference stay out of the tree. The gates hold every ELF to the glibc floor
# and resolve each NEEDED library inside the tree or on the declared host
# libraries. A copy of the static launcher replaces each makeWrapper script and
# applies the same environment relative to wherever the tree is.
{ pkgs, eonPackage, wrappers, wrapperArgs }:
let
  inherit (pkgs) lib;
  excluded = [ pkgs.glibc pkgs.mesa ];
  shipped = path: !lib.any (package: lib.hasPrefix "${package}/" path) excluded;
  # Prefix entries in excluded paths leave with them; the tree gate requires every other value.
  bundleEnvironment = environment: {
    set = environment.set or { };
    setDefault = environment.setDefault or { };
    prefix = lib.filterAttrs (_: paths: paths != [ ]) (
      lib.mapAttrs (_: lib.filter shipped) (environment.prefix or { })
    );
  };
  bundleWrappers = map (
    wrapper:
    wrapper
    // {
      environment = bundleEnvironment wrapper.environment;
      target = "${dirOf wrapper.program}/.${baseNameOf wrapper.program}-wrapped";
    }
  ) wrappers;
  paths =
    wrapper:
    lib.attrValues (wrapper.environment.set // wrapper.environment.setDefault)
    ++ lib.concatLists (lib.attrValues wrapper.environment.prefix);

  # The launcher prefixes each store path with the tree root, so a value must be a store
  # path that needs no escaping in Rust and holds no separator.
  literal =
    value:
    assert lib.assertMsg (builtins.match "[A-Za-z0-9+._?=/-]+" value != null)
      "launcher table value needs escaping: ${value}";
    ''"${value}"'';
  storePath =
    value:
    assert lib.assertMsg (lib.hasPrefix "${builtins.storeDir}/" value)
      "launcher table value is not a store path: ${value}";
    literal value;
  pairs = render: attrs: "&[${lib.concatStrings (lib.mapAttrsToList (name: value: "(${literal name}, ${render value}), ") attrs)}]";
  table = pkgs.writeText "eon-launcher-table.rs" ''
    const WRAPPERS: &[Wrapper] = &[
    ${lib.concatMapStrings (wrapper: ''
      Wrapper {
          program: ${storePath wrapper.program},
          target: ${storePath wrapper.target},
          set: ${pairs storePath wrapper.environment.set},
          set_default: ${pairs storePath wrapper.environment.setDefault},
          prefix: ${pairs (paths: "&[${lib.concatMapStringsSep ", " storePath paths}]") wrapper.environment.prefix},
      },
    '') bundleWrappers}];
  '';
  launcher = pkgs.pkgsStatic.rustPlatform.buildRustPackage {
    pname = "eon-launcher";
    version = (builtins.fromTOML (builtins.readFile ../launcher/Cargo.toml)).package.version;
    src = lib.fileset.toSource {
      root = ../launcher;
      fileset = lib.fileset.unions [
        ../launcher/Cargo.toml
        ../launcher/Cargo.lock
        ../launcher/src
      ];
    };
    cargoLock.lockFile = ../launcher/Cargo.lock;
    EON_LAUNCHER_TABLE = table;
  };

  relink = pkgs.rustPlatform.buildRustPackage {
    pname = "eon-relink";
    version = (builtins.fromTOML (builtins.readFile ../relink/Cargo.toml)).package.version;
    src = lib.fileset.toSource {
      root = ../relink;
      fileset = lib.fileset.unions [
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
  tree = pkgs.runCommand "eon-bundle-relink" {
    nativeBuildInputs = [ relink pkgs.binutils pkgs.patchelf ];
  } ''
    hostLibraries=(--host-library libgcc_s.so.1 --host-library libwayland-client.so.0)
    for library in ${pkgs.glibc}/lib/*.so.*; do
      hostLibraries+=(--host-library "''${library##*/}")
    done
    eon-relink --registration ${closure}/registration --root ${eonPackage} \
      ${lib.concatMapStringsSep " " (package: "--exclude ${package}") excluded} \
      --glibc-floor 2.42 "''${hostLibraries[@]}" "$out"

    if [[ "$(readelf -l ${launcher}/bin/eon-launcher)" == *INTERP* ]]; then
      echo "the launcher must be static" >&2
      exit 1
    fi
    ${lib.concatMapStrings (wrapper: ''
      grep -qF 'exec -a "$0" "${wrapper.target}"' "$out${wrapper.program}"
      install -m 555 ${launcher}/bin/eon-launcher "$out${wrapper.program}"
    '') bundleWrappers}
    if grep -rlF 'exec -a "$0"' "$out/nix/store"; then
      echo "makeWrapper scripts remain outside the launcher table" >&2
      exit 1
    elif test $? != 1; then
      exit 1
    fi
    for path in ${lib.escapeShellArgs (lib.concatMap paths bundleWrappers)}; do
      if [[ ! -e "$out$path" ]]; then
        echo "launcher value is not in the tree: $path" >&2
        exit 1
      fi
    done
  '';

  # A host without Nix as far as the tree can tell: this nixpkgs glibc stands in for the
  # host glibc and its loader sits at the psABI path; no other store path is visible. The
  # tree needs no namespace; the check uses bwrap only to build that host.
  host = lib.escapeShellArgs [
    "--ro-bind" "${pkgs.glibc}" "${pkgs.glibc}"
    "--ro-bind" "${pkgs.glibc}/lib/ld-linux-x86-64.so.2" "/lib64/ld-linux-x86-64.so.2"
    "--ro-bind" "${pkgs.pkgsStatic.strace}/bin/strace" "/strace"
    "--dev" "/dev"
    "--proc" "/proc"
    "--clearenv"
  ];
  busybox = "${pkgs.pkgsStatic.busybox}/bin/busybox";
  # Inputs for comparing each launcher with makeWrapper as NAME VALUE pairs: unset, empty,
  # and set to a value the launcher must replace, keep or move to the front. @ROOT@ stands
  # for the tree root.
  cases =
    environment:
    let
      assigned = environment.set // environment.setDefault;
    in
    [
      [ ]
      (lib.concatMap (name: [ name "" ]) (lib.attrNames (assigned // environment.prefix)))
      (lib.concatLists (
        lib.mapAttrsToList (name: _: [ name "/custom" ]) assigned
        ++ lib.mapAttrsToList (name: paths: [ name "/x:@ROOT@${lib.last paths}:/y" ]) environment.prefix
      ))
    ];
in
{
  inherit tree;
  launch = pkgs.runCommand "eon-bundle-launch" {
    nativeBuildInputs = [ pkgs.bubblewrap pkgs.makeWrapper ];
  } ''
    mkdir home
    # Runs COMMAND with the tree at ROOT into the file actual, and fails on any store path
    # the trace touches outside the stand-in host glibc.
    run() {
      local root=$1
      shift
      bwrap ${host} --ro-bind ${tree} "$root" --bind "$PWD/home" /home/eon \
        --setenv HOME /home/eon --setenv PATH "$root${eonPackage}/libexec/eon/bin" \
        -- /strace -f -qq -s 4096 -e trace=%file -o /home/eon/trace "$@" > actual
      if grep -F '"${builtins.storeDir}/' home/trace | grep -vF '"${pkgs.glibc}/'; then
        echo "the tree looked up a store path" >&2
        exit 1
      fi
    }
    for root in /opt/eon /srv/moved/eon; do
      for arguments in versions --help; do
        run "$root" "$root${eonPackage}/bin/eon" $arguments
        ${eonPackage}/bin/eon $arguments | cmp - actual
      done
      for command in nu bash eon-nu eon-bash; do
        run "$root" "$command" --version
        HOME="$PWD/home" ${eonPackage}/libexec/eon/bin/$command --version | cmp - actual
      done
    done

    # Each launcher must hand its target the environment makeWrapper builds from the same
    # definition, with store paths under the tree root. busybox stands in for the target
    # and prints what it receives.
    # The shell and bwrap add these themselves.
    received() {
      grep -Ev '^(_|SHLVL|PWD|OLDPWD)=' | sort
    }
    compare() {
      local program=$1 target=$2 root=/opt/eon reference=() launched=()
      shift 2
      while (($#)); do
        reference+=("$1=''${2//@ROOT@/}")
        launched+=(--setenv "$1" "''${2//@ROOT@/$root}")
        shift 2
      done
      env -i "''${reference[@]}" "$PWD/env" |
        sed "s|${builtins.storeDir}/|$root${builtins.storeDir}/|g" | received > expected
      bwrap ${host} --ro-bind ${tree} "$root" --ro-bind ${busybox} "$root$target" \
        "''${launched[@]}" --argv0 env -- "$root$program" | received > actual
      diff -u expected actual
    }
    ${lib.concatMapStrings (wrapper: ''
      makeWrapper ${busybox} env --inherit-argv0 ${wrapperArgs wrapper.environment}
      ${lib.concatMapStrings (inputs: ''
        compare ${lib.escapeShellArgs ([ wrapper.program wrapper.target ] ++ inputs)}
      '') (cases wrapper.environment)}
    '') bundleWrappers}
    touch "$out"
  '';
}
