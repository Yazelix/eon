# `eon-test`: runs the Rust test suites so they cannot reach a live Eon
# session, with only the toolchain variables of the calling `nix develop`
# shell, plus the variables that point Orbit's tests at a server.
#
# On Linux the suites run inside bubblewrap, with private PID, network and
# /dev, and empty /tmp and /run that hide the session's Wayland, D-Bus and Eon
# sockets. Without arguments it runs the workspace suites, then the
# installer's, stopping at the first failing suite.
#
# macOS has no such namespaces, so the suites share the host's processes,
# network and files. There eon-test refuses while this user runs an Eon
# supervisor and gives the suites a fresh private HOME, TMPDIR and runtime
# root. Without arguments it runs the workspace suites; the installer is
# Linux-only.
{ pkgs }:
let
  macos = ''
    if [ -z "''${GHOSTTY_SOURCE_DIR:-}" ] || [ -z "''${EON_VENUS_FONT_DIRS:-}" ] || ! command -v cargo >/dev/null; then
      echo "eon-test: the dev shell toolchain is missing; run it as nix develop -c eon-test" >&2
      exit 1
    fi
    # A suite that scans or signals processes could reach a live Eon, so
    # refuse while this user runs a supervisor: eon or eonbase, or the
    # executable that makeWrapper names .eon-wrapped. Inside an Eon terminal
    # the supervisor can be an ancestor, which pgrep skips without -a.
    running=0
    /usr/bin/pgrep -a -U "$(id -u)" -x '\.?eon(base)?(-wrapped)?' >/dev/null || running=$?
    case $running in
      0)
        echo "eon-test: an Eon supervisor of this user is running, or one an earlier run left behind; quit it first, since macOS cannot isolate the suites from its processes" >&2
        exit 1
        ;;
      1) ;;
      *)
        echo "eon-test: cannot list this user's processes to check for a running Eon" >&2
        exit 1
        ;;
    esac
    echo "eon-test: macOS has no namespaces; the suites share this machine's processes, network and files, with only HOME, TMPDIR and the runtime root private" >&2

    # macOS socket paths hold 103 bytes, so the suites' runtime roots need a
    # TMPDIR of at most 7 bytes, and a fresh one, so that stale test
    # directories cannot lengthen them.
    private=
    for name in {0..9} {a..z}; do
      if mkdir -m 700 "/tmp/e$name" 2>/dev/null; then
        private=/tmp/e$name
        break
      fi
    done
    if [ -z "$private" ]; then
      echo "eon-test: no free private directory /tmp/e<0-9a-z>; remove stale ones" >&2
      exit 1
    fi
    trap 'rm -rf "$private"' EXIT
    # Venus's tests build their font database without EON_VENUS_FONT_DIRS,
    # and on macOS it reads only the system and user font folders.
    mkdir -p "$private/home/Library/Fonts"
    IFS=: read -ra fonts <<< "$EON_VENUS_FONT_DIRS"
    ln -s "''${fonts[@]}" "$private/home/Library/Fonts/"
    # Without XDG_RUNTIME_DIR, Eon's macOS runtime root is this user's shared
    # /tmp/eon-<uid>.
    mkdir -m 700 "$private/r"

    root=$(git rev-parse --show-toplevel)
    cd "$root"
    export CARGO_HOME="''${CARGO_HOME:-$HOME/.cargo}"
    cargo fetch --locked

    # Inside an Eon terminal, Eon's managed command directory shadows tools
    # such as bash and nu; keep it off the suites' PATH.
    path=
    IFS=: read -ra entries <<< "$PATH"
    for entry in "''${entries[@]}"; do
      case $entry in
        */libexec/eon/bin) ;;
        *) path=''${path:+$path:}$entry ;;
      esac
    done

    environment=(PATH="$path" HOME="$private/home" TMPDIR="$private" XDG_RUNTIME_DIR="$private/r")
    while read -r -d "" entry; do
      name=''${entry%%=*}
      case $name in
        CARGO_* | RUST* | NIX_* | PKG_CONFIG* | CC | CXX | AR | AS | LD | NM | \
          OBJCOPY | OBJDUMP | RANLIB | READELF | SIZE | STRINGS | STRIP | \
          GHOSTTY_* | LIBGHOSTTY_* | ZIG_* | BEND_* | EON_VENUS_FONT_DIRS | \
          SDKROOT* | DEVELOPER_DIR* | MACOSX_DEPLOYMENT_TARGET* | \
          ORBIT_* | MISMATCHED_WORKER | \
          LANG | LC_ALL | USER | LOGNAME | SOURCE_DATE_EPOCH)
          environment+=("$entry")
          ;;
      esac
    done < <(env -0)

    isolated() {
      env -i "''${environment[@]}" "$@"
    }

    # Eon Runtime's lease tests run the Orbit at ORBIT_BIN: this workspace's,
    # unless the caller names another.
    if [ -z "''${ORBIT_BIN:-}" ]; then
      isolated cargo build --locked --offline --package yazelix-orbit
      target=''${CARGO_TARGET_DIR:-''${CARGO_BUILD_TARGET_DIR:-target}}
      case $target in
        /*) ;;
        *) target=$root/$target ;;
      esac
      environment+=(ORBIT_BIN="$target/debug/yazelix-orbit")
    fi

    if [ "$#" -gt 0 ]; then
      isolated cargo test --locked --offline "$@"
    else
      isolated cargo test --locked --offline --workspace
    fi
  '';
in
pkgs.writeShellApplication {
  name = "eon-test";
  runtimeInputs = pkgs.lib.optional pkgs.stdenv.isLinux pkgs.bubblewrap ++ [
    pkgs.coreutils
    pkgs.git
  ];
  text = if pkgs.stdenv.isLinux then ''
    if [ -z "''${GHOSTTY_SOURCE_DIR:-}" ] || [ -z "''${FONTCONFIG_FILE:-}" ] || ! command -v cargo >/dev/null; then
      echo "eon-test: the dev shell toolchain is missing; run it as nix develop -c eon-test" >&2
      exit 1
    fi
    if ! bwrap --unshare-all --ro-bind / / true; then
      echo "eon-test: bubblewrap cannot create a sandbox; unprivileged user namespaces are required" >&2
      exit 1
    fi

    root=$(git rev-parse --show-toplevel)
    cd "$root"
    export CARGO_HOME="''${CARGO_HOME:-$HOME/.cargo}"
    mkdir -p "$CARGO_HOME"
    writable=(--bind "$root" "$root" --bind "$CARGO_HOME" "$CARGO_HOME")
    for target in "''${CARGO_TARGET_DIR:-}" "''${CARGO_BUILD_TARGET_DIR:-}"; do
      if [ -n "$target" ]; then
        # Cargo resolves a relative target directory against the repository.
        case $target in
          /*) ;;
          *) target=$root/$target ;;
        esac
        mkdir -p "$target"
        writable+=(--bind "$target" "$target")
      fi
    done

    # The sandbox has no network, so fetch the locked crates first.
    cargo fetch --locked
    cargo fetch --locked --manifest-path installer/Cargo.toml

    # Inside an Eon terminal, Eon's managed command directory shadows tools
    # such as bash and nu; keep it off the sandbox PATH.
    path=
    IFS=: read -ra entries <<< "$PATH"
    for entry in "''${entries[@]}"; do
      case $entry in
        */libexec/eon/bin) ;;
        *) path=''${path:+$path:}$entry ;;
      esac
    done

    environment=(--clearenv --setenv PATH "$path")
    while read -r -d "" entry; do
      name=''${entry%%=*}
      case $name in
        CARGO_* | RUST* | NIX_* | PKG_CONFIG* | CC | CXX | AR | AS | LD | NM | \
          OBJCOPY | OBJDUMP | RANLIB | READELF | SIZE | STRINGS | STRIP | \
          GHOSTTY_* | LIBGHOSTTY_* | ZIG_* | BEND_* | FONTCONFIG_FILE | \
          ORBIT_* | MISMATCHED_WORKER | \
          LANG | LC_ALL | USER | LOGNAME | SOURCE_DATE_EPOCH)
          environment+=(--setenv "$name" "''${entry#*=}")
          ;;
      esac
    done < <(env -0)

    sandbox() {
      bwrap --die-with-parent --new-session --unshare-all \
        --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --tmpfs /run \
        "''${writable[@]}" --dir /tmp/home \
        --perms 0700 --dir /tmp/run \
        "''${environment[@]}" --setenv HOME /tmp/home --setenv XDG_RUNTIME_DIR /tmp/run \
        --chdir "$root" -- "$@"
    }

    # Eon Runtime's lease tests run the Orbit at ORBIT_BIN: this workspace's,
    # unless the caller names another.
    if [ -z "''${ORBIT_BIN:-}" ]; then
      sandbox cargo build --locked --offline --package yazelix-orbit
      target=''${CARGO_TARGET_DIR:-''${CARGO_BUILD_TARGET_DIR:-target}}
      case $target in
        /*) ;;
        *) target=$root/$target ;;
      esac
      environment+=(--setenv ORBIT_BIN "$target/debug/yazelix-orbit")
    fi

    if [ "$#" -gt 0 ]; then
      sandbox cargo test --locked --offline "$@"
    else
      sandbox cargo test --locked --offline --workspace
      sandbox cargo test --locked --offline --manifest-path installer/Cargo.toml
    fi
  '' else macos;
}
