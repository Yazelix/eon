# The eon closure relinked to run on the host glibc without a namespace: every
# program interpreter becomes the x86-64 psABI path, store RUNPATH entries
# become $ORIGIN-relative, and absolute store symlinks become relative. The host
# supplies glibc, the Vulkan driver and Venus's Wayland client library, so glibc,
# Mesa, Wayland and what only they reference stay out of the tree. The gates hold
# every ELF to the glibc floor and resolve each NEEDED library inside the tree or
# on the declared host libraries. A copy of the static launcher replaces each
# makeWrapper script and applies the same environment relative to wherever the
# tree is.
#
# Each wrapper may add `relocated`, environment for the bundle only that names
# data roots its programs otherwise find through compiled-in store paths, and
# `hostRuntime`, when its program loads host drivers and so must take the
# libraries it links from the host. Each fallback is a link where a program
# looks when its compiled-in store path, the target, is missing.
#
# Each substitute ships a rebuild of a closure path under that path's name, for a
# library the floor would otherwise reject.
{ pkgs, eonPackage, wrappers, wrapperArgs, fallbacks }:
let
  inherit (pkgs) lib;
  excluded = [ pkgs.glibc pkgs.mesa pkgs.wayland ];
  glibcFloor = "2.39";
  substitutes = [
    {
      # ncurses built on glibc 2.42 calls that release's cfgetospeed.
      original = pkgs.ncurses;
      replacement = pkgs.ncurses.overrideAttrs {
        NIX_CFLAGS_COMPILE = "-include ${./ncurses-termios-speed.h}";
      };
    }
  ];
  shipped = path: !lib.any (package: lib.hasPrefix "${package}/" path) excluded;
  # Prefix entries in excluded paths leave with them; the tree gate requires every other value.
  bundleEnvironment =
    { environment, relocated ? { }, ... }:
    {
      set = environment.set or { } // relocated.set or { };
      setDefault = environment.setDefault or { } // relocated.setDefault or { };
      prefix = lib.filterAttrs (_: paths: paths != [ ]) (
        lib.mapAttrs (_: lib.filter shipped) (
          lib.zipAttrsWith (_: lib.concatLists) [
            (environment.prefix or { })
            (relocated.prefix or { })
          ]
        )
      );
    };
  bundleWrappers = map (
    wrapper:
    wrapper
    // {
      environment = bundleEnvironment wrapper;
      target = "${dirOf wrapper.program}/.${baseNameOf wrapper.program}-wrapped";
    }
  ) wrappers;
  paths =
    environment:
    lib.attrValues (environment.set or { } // environment.setDefault or { })
    ++ lib.concatLists (lib.attrValues (environment.prefix or { }));
  # The compiled-in store paths the tree stands in for, which programs may still probe.
  replaced =
    map (fallback: fallback.target) fallbacks
    ++ lib.concatMap (wrapper: paths (wrapper.relocated or { })) wrappers;

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
      ${lib.concatMapStrings (wrapper: "--host-runtime ${wrapper.target} ") (
        lib.filter (wrapper: wrapper.hostRuntime or false) bundleWrappers
      )} \
      ${lib.concatMapStrings ({ original, replacement }: "--substitute ${original}=${replacement} ") substitutes} \
      --glibc-floor ${glibcFloor} "''${hostLibraries[@]}" "$out"
    ${lib.concatMapStrings ({ link, target }: ''
      ln -s "$(realpath -s -m --relative-to="$out${dirOf link}" "$out${target}")" "$out${link}"
      test -d "$out${link}/"
    '') fallbacks}

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
    for path in ${lib.escapeShellArgs (lib.concatMap (wrapper: paths wrapper.environment) bundleWrappers)}; do
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
  inherit tree glibcFloor;
  excluded = map toString excluded;
  substituted = map (lib.mapAttrs (_: toString)) substitutes;
  launch = pkgs.runCommand "eon-bundle-launch" {
    nativeBuildInputs = [ pkgs.bubblewrap pkgs.makeWrapper ];
  } ''
    mkdir home
    # Runs COMMAND with the tree at ROOT into the file actual, and fails on any store path
    # the trace touches outside the stand-in host glibc and the replaced compiled-in paths.
    run() {
      local root=$1
      shift
      bwrap ${host} --ro-bind ${tree} "$root" --bind "$PWD/home" /home/eon \
        --setenv HOME /home/eon --setenv PATH "$root${eonPackage}/libexec/eon/bin" \
        -- /strace -f -qq -s 4096 -e trace=%file -o /home/eon/trace "$@" > actual
      if grep -F '"${builtins.storeDir}/' home/trace |
        grep -vF ${lib.concatMapStringsSep " " (path: "-e '\"${path}'") ([ "${pkgs.glibc}/" ] ++ replaced)}; then
        echo "the tree looked up a store path" >&2
        exit 1
      fi
    }
    # Runs the managed COMMAND from the Nix product into the file expected.
    reference() {
      HOME="$PWD/home" ${eonPackage}/libexec/eon/bin/"$@" > expected
    }
    terminfo='for term in eon xterm-256color; do "$0" -T $term colors; done'
    for root in /opt/eon /srv/moved/eon; do
      for arguments in versions --help; do
        run "$root" "$root${eonPackage}/bin/eon" $arguments
        ${eonPackage}/bin/eon $arguments | cmp - actual
      done
      for command in nu bash eon-nu eon-bash; do
        run "$root" "$command" --version
        reference "$command" --version
        cmp expected actual
      done
      # Helix loads a grammar and its highlight queries from its runtime.
      run "$root" hx --health bash
      test "$(grep -cE '^(Tree-sitter parser|Highlight queries): .*✓' actual)" = 2
      # ncurses finds Orbit's terminal type and its own.
      run "$root" eon-bash -c "$terminfo" "$root${pkgs.ncurses}/bin/tput"
      reference eon-bash -c "$terminfo" ${pkgs.ncurses}/bin/tput
      cmp expected actual
      # zoxide runs fzf from the PATH; the filter keeps fzf off the terminal.
      run "$root" eon-bash -c "zoxide add $root && _ZO_FZF_OPTS=--filter=$root zoxide query --interactive"
      test "$(cat actual)" = "$root"
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
