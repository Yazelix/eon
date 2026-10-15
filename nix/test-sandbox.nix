# `eon-test`: runs the Rust test suites inside bubblewrap so they cannot reach
# a live Eon session. The sandbox has private PID, network and /dev, empty
# /tmp and /run that hide the session's Wayland, D-Bus and Eon sockets, and
# only the toolchain variables of the calling `nix develop` shell. Without arguments it
# runs the workspace suites, then the installer's, stopping at the first
# failing suite.
{ pkgs }:
pkgs.writeShellApplication {
  name = "eon-test";
  runtimeInputs = [
    pkgs.bubblewrap
    pkgs.coreutils
    pkgs.git
  ];
  text = ''
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
          GHOSTTY_* | LIBGHOSTTY_* | ZIG_* | FONTCONFIG_FILE | \
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

    if [ "$#" -gt 0 ]; then
      sandbox cargo test --locked --offline "$@"
    else
      sandbox cargo test --locked --offline --workspace
      sandbox cargo test --locked --offline --manifest-path installer/Cargo.toml
    fi
  '';
}
