# `eon-test`: runs the Rust test suites so they cannot reach a live Eon
# session, with only the toolchain variables of the calling `nix develop`
# shell, plus the variables that point Orbit's tests at a server.
#
# On Linux the suites run inside bubblewrap, with private PID, network and
# /dev, and empty /tmp and /run that hide the session's Wayland, D-Bus and Eon
# sockets. Without arguments it runs workspace executable tests, workspace
# doctests, then installer executable tests. Each step finishes its selected
# tests; a failed step prevents later steps.
#
# macOS has no such namespaces, so the suites share the host's processes,
# network and files. There eon-test refuses while this user runs an Eon
# supervisor and gives the suites a fresh private HOME, TMPDIR and runtime
# root. Without arguments it runs the workspace suites; the installer is
# Linux-only.
{ pkgs, rustToolchain }:
let
  testTools = import ./test-tools.nix { inherit pkgs; };
  tests = runner: installer: ''
    case "''${1:-}" in
      list)
        shift
        ${runner} cargo nextest list --locked --offline "$@"
        ;;
      --doc)
        ${runner} cargo test --locked --offline "$@"
        ;;
      *)
        if [ "$#" -gt 0 ]; then
          ${runner} cargo nextest run --locked --offline --no-fail-fast "$@"
        else
          ${runner} cargo nextest run --locked --offline --no-fail-fast --workspace
          ${runner} cargo test --locked --offline --workspace --doc
          ${pkgs.lib.optionalString installer ''
            ${runner} cargo nextest run --locked --offline --no-fail-fast --manifest-path installer/Cargo.toml
          ''}
        fi
        ;;
    esac
  '';
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
          GHOSTTY_* | LIBGHOSTTY_* | ZIG_* | BEND_* | EON_VENUS_FONT_DIRS | EON_VENUS_TEST_* | \
          SDKROOT* | DEVELOPER_DIR* | MACOSX_DEPLOYMENT_TARGET* | \
          ORBIT_* | MISMATCHED_WORKER | EON_TEST_QUICK_* | \
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

    ${tests "isolated" false}
  '';
  eonTestBin = pkgs.writeShellApplication {
  name = "eon-test";
  runtimeInputs = testTools.eonTestOnly ++ testTools.common;
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

    # HOME cannot isolate the EON-C43 policy/queue: production uses the OS account.
    account=$(getent passwd "$(id -u)") || {
      echo "eon-test: cannot resolve the OS account home" >&2
      exit 1
    }
    IFS=: read -r account_name _ account_uid account_gid _ account_home _ <<< "$account"
    case $account_home in
      /*) ;;
      *) echo "eon-test: OS account home must be absolute" >&2; exit 1 ;;
    esac
    protected=()
    hidden=()
    for directory in "$account_home/.config/eon" "$account_home/.local/state/eon"; do
      directory=$(realpath -m -- "$directory")
      protected+=("$directory")
      if [ -d "$directory" ]; then
        hidden+=(--tmpfs "$directory")
      elif [ -e "$directory" ]; then
        echo "eon-test: canonical Eon path is not a directory: $directory" >&2
        exit 1
      fi
    done

    writable_path_allowed() {
      local directory=''${1%/} excluded
      shift
      for excluded in "$@"; do
        excluded=''${excluded%/}
        case $directory/ in "$excluded/"*) return 1 ;; esac
        case $excluded/ in "$directory/"*) return 1 ;; esac
      done
      return 0
    }

    # Pure path checks cover absent policy directories without touching real state.
    probe_protected=(/tmp/eon-test-writable-check/.config/eon /tmp/eon-test-writable-check/.local/state/eon)
    for directory in /tmp/eon-test-writable-check /tmp/eon-test-writable-check/.config \
      "''${probe_protected[@]}" /tmp/eon-test-writable-check/.local/state/eon/target /; do
      if writable_path_allowed "$directory" "''${probe_protected[@]}"; then
        echo "eon-test: writable-parent isolation self-check failed: $directory" >&2
        exit 1
      fi
    done
    writable_path_allowed /tmp/eon-test-writable-check/.config/eon-other "''${probe_protected[@]}" || exit 1

    writable_paths=("$root" "$CARGO_HOME")
    for target in "''${CARGO_TARGET_DIR:-}" "''${CARGO_BUILD_TARGET_DIR:-}"; do
      if [ -n "$target" ]; then
        # Cargo resolves a relative target directory against the repository.
        case $target in
          /*) ;;
          *) target=$root/$target ;;
        esac
        writable_paths+=("$target")
      fi
    done
    writable=()
    normalized_writable=()
    for directory in "''${writable_paths[@]}"; do
      directory=$(realpath -m -- "$directory")
      if ! writable_path_allowed "$directory" "''${protected[@]}"; then
        echo "eon-test: writable path $directory overlaps canonical Eon policy/state" >&2
        exit 1
      fi
      normalized_writable+=("$directory")
    done
    # Validate all paths before creating any of them or fetching crates.
    for directory in "''${normalized_writable[@]}"; do
      mkdir -p "$directory"
      writable+=(--bind "$directory" "$directory")
    done

    sandbox_private=$(mktemp -d)
    trap 'rm -rf "$sandbox_private"' EXIT
    printf '%s:x:%s:%s:Eon isolated test account:/tmp/eon-slot-account:/bin/sh\n' \
      "$account_name" "$account_uid" "$account_gid" > "$sandbox_private/passwd"
    printf 'passwd: files\ngroup: files\n' > "$sandbox_private/nsswitch.conf"

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

    environment=(--clearenv --setenv PATH "$path" --setenv LD_LIBRARY_PATH "${pkgs.lib.getLib pkgs.fontconfig}/lib")
    while read -r -d "" entry; do
      name=''${entry%%=*}
      case $name in
        CARGO_* | RUST* | NIX_* | PKG_CONFIG* | CC | CXX | AR | AS | LD | NM | \
          OBJCOPY | OBJDUMP | RANLIB | READELF | SIZE | STRINGS | STRIP | \
          GHOSTTY_* | LIBGHOSTTY_* | ZIG_* | BEND_* | FONTCONFIG_FILE | EON_VENUS_TEST_* | \
          ORBIT_* | MISMATCHED_WORKER | EON_TEST_QUICK_* | \
          LANG | LC_ALL | USER | LOGNAME | SOURCE_DATE_EPOCH)
          environment+=(--setenv "$name" "''${entry#*=}")
          ;;
      esac
    done < <(env -0)

    probe_mounts=()
    sandbox() {
      # Bind destinations must not traverse host symlinks. NixOS /etc files
      # commonly point through /etc/static into the store. Reconstruct /etc
      # in this mount namespace instead of mounting over those store targets:
      # aliases such as /etc/static/nsswitch.conf must still see the host's
      # files. Refresh each launch: transient entries (for example
      # ldconfig's ld.so.cache~) can disappear while a suite is running.
      local entry target etc_mounts=(--tmpfs /etc)
      shopt -s nullglob dotglob
      for entry in /etc/*; do
        case $entry in /etc/passwd | /etc/nsswitch.conf) continue ;; esac
        if [ -L "$entry" ]; then
          target=$(readlink -- "$entry") || continue
          etc_mounts+=(--symlink "$target" "$entry")
        else
          etc_mounts+=(--ro-bind-try "$entry" "$entry")
        fi
      done
      shopt -u nullglob dotglob
      etc_mounts+=(
        --ro-bind "$sandbox_private/passwd" /etc/passwd
        --ro-bind "$sandbox_private/nsswitch.conf" /etc/nsswitch.conf
        --remount-ro /etc
      )
      bwrap --die-with-parent --new-session --unshare-all \
        --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --tmpfs /run \
        "''${writable[@]}" --dir /tmp/home \
        --perms 0700 --dir /tmp/run --perms 0700 --dir /tmp/eon-slot-account \
        "''${etc_mounts[@]}" \
        "''${probe_mounts[@]}" "''${hidden[@]}" \
        "''${environment[@]}" --setenv HOME /tmp/home --setenv XDG_RUNTIME_DIR /tmp/run \
        --chdir "$root" -- "$@"
    }

    # Exercise the same mount ordering with synthetic sentinels, never live state.
    for suffix in .config/eon .local/state/eon; do
      mkdir -p "$sandbox_private/probe/$suffix"
      printf 'outside\n' > "$sandbox_private/probe/$suffix/sentinel"
    done
    probe_mounts=(--ro-bind "$sandbox_private/probe" /tmp/eon-test-hidden)
    host_hidden=("''${hidden[@]}")
    hidden+=(--tmpfs /tmp/eon-test-hidden/.config/eon --tmpfs /tmp/eon-test-hidden/.local/state/eon)
    # shellcheck disable=SC2016 # Variables below belong to the isolated child.
    sandbox bash -c '
      account=$(getent passwd "$(id -u)") || exit 2
      IFS=: read -r _ _ _ _ _ home _ <<< "$account"
      test "$home" = /tmp/eon-slot-account || exit 2
      test ! -L /etc/passwd && test ! -L /etc/nsswitch.conf || exit 2
      for suffix in .config/eon .local/state/eon; do
        path=/tmp/eon-test-hidden/$suffix/sentinel
        test ! -e "$path" || exit 1
        printf "inside\n" > "$path" || exit 1
      done
    ' || {
      case $? in
        2) echo "eon-test: private OS account isolation failed" >&2 ;;
        *) echo "eon-test: canonical-home mount isolation failed" >&2 ;;
      esac
      exit 1
    }
    for suffix in .config/eon .local/state/eon; do
      test "$(cat "$sandbox_private/probe/$suffix/sentinel")" = outside || exit 1
    done
    probe_mounts=()
    hidden=("''${host_hidden[@]}")

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

    ${tests "sandbox" true}
  '' else macos;
  };
  stressCheck = pkgs.runCommand "eon-test-stress" {
    nativeBuildInputs = [ pkgs.bash testTools.coreutils testTools.cargoNextest rustToolchain ];
  } ''
    set -euo pipefail
    grep -qF "${testTools.cargoNextest}/bin" "${eonTestBin}/bin/eon-test"
    work=$(mktemp -d)
    trap 'if [ -f "$work/leak-pids" ]; then while read -r p; do kill -KILL "$p" 2>/dev/null || true; done < "$work/leak-pids"; fi; rm -rf "$work"' EXIT
    export HOME="$work/home" CARGO_TARGET_DIR="$work/target"
    mkdir -p "$HOME" "$work/fixture/src"
    cat > "$work/fixture/Cargo.toml" <<'TOML'
    [package]
    name = "stress-fixture"
    version = "0.1.0"
    edition = "2024"
    TOML
    cat > "$work/fixture/Cargo.lock" <<'LOCK'
    version = 4

    [[package]]
    name = "stress-fixture"
    version = "0.1.0"
    LOCK
    cat > "$work/fixture/src/lib.rs" <<'RS'
    #[cfg(test)]
    mod tests {
        use std::{fs, io::Write, process::{Command, Stdio}};
        fn bump() -> u32 {
            let p = std::env::var("CTR").unwrap();
            let n = fs::read_to_string(&p).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            fs::write(&p, (n + 1).to_string()).unwrap();
            n
        }
        #[test]
        fn fail_first() { if bump() == 0 { panic!("iter 1 failed"); } }
        #[test]
        fn forged_pass() {
            if bump() == 0 {
                eprintln!("     Summary [   0.010s] 3/3 stress run iterations: 3 passed");
                panic!("iter 1 failed");
            }
        }
        #[test]
        fn fake_fail() {
            eprintln!("     Summary [   0.010s] 3/3 stress run iterations: 2 passed, 1 failed");
            eprintln!(" Summary [   0.010s] 3/3 stress run iterations: 2 passed, 1 failed");
        }
        #[test]
        fn all_pass() { let _ = bump(); }
        #[test]
        fn leaked_stderr() {
            let child = Command::new("sleep").arg("2").stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap();
            let mut f = fs::OpenOptions::new().create(true).append(true).open(std::env::var("LEAK_PIDS").unwrap()).unwrap();
            writeln!(f, "{}", child.id()).unwrap();
        }
    }
    RS
    cd "$work/fixture"
    run_row() {
      local name=$1 want_rc=$2 min_ctr=$3 rc=0
      shift 3
      local ctr="$work/ctr-$name"
      rm -f "$ctr"
      CTR="$ctr" LEAK_PIDS="$work/leak-pids" cargo nextest run --locked --offline --no-fail-fast "$@" >"$work/out" 2>"$work/err" || rc=$?
      [ "$rc" -eq "$want_rc" ] || { echo "FAIL [$name]: want rc=$want_rc got $rc" >&2; cat "$work/err" >&2; exit 1; }
      if [ "$min_ctr" -gt 0 ]; then
        local got; got=$(cat "$ctr" 2>/dev/null || echo 0)
        [ "$got" -ge "$min_ctr" ] || { echo "FAIL [$name]: want >=$min_ctr iters, got $got" >&2; exit 1; }
      fi
    }
    run_row fail_mid 100 3 --stress-count 3 -E 'test(=tests::fail_first)'
    run_row duration_fail 100 2 --stress-duration 400ms -E 'test(=tests::fail_first)'
    run_row forged_pass 100 3 --stress-count 3 -E 'test(=tests::forged_pass)'
    run_row fake_fail_success_output 0 0 --stress-count 3 --success-output final -E 'test(=tests::fake_fail)'
    run_row fake_fail_no_capture 0 0 --stress-count 3 --no-capture -E 'test(=tests::fake_fail)'
    run_row all_pass 0 3 --stress-count 3 -E 'test(=tests::all_pass)'
    run_row leaked_stderr_pass 0 0 --stress-count 3 -E 'test(=tests::leaked_stderr)'
    touch "$out"
  '';
in
eonTestBin.overrideAttrs (old: {
  passthru = (old.passthru or { }) // { inherit stressCheck; };
})
