# Orbit's checks (orbit/docs/DEVELOPMENT.md): the Bend laws, the generated
# boundary, the terminfo entry, and the real-PTY checks. Each real-PTY check
# runs one test of the package's real-PTY test binary (orbit/tests/real_pty)
# against the package; the differential check also runs the old Rust Orbit,
# built from oldOrbitSource.
{
  pkgs,
  bend,
  orbitPackage,
  ghosttyBuildEnv,
  workspaceCargoLock,
  oldOrbitSource,
}:
let
  inherit (pkgs) lib;
  inherit (lib) fileset;
  bendFiles = fileset.fileFilter (file: file.hasExt "bend") ../orbit/src;
  boundarySource = fileset.toSource {
    root = ../orbit;
    fileset = fileset.unions [
      ../orbit/tools/boundary.rs
      ../orbit/src/boundary.schema
      ../orbit/src/boundary.rs
      ../orbit/src/boundary.bend
    ];
  };
  workerSource = fileset.toSource {
    root = ../orbit;
    fileset = bendFiles;
  };
  lawSource = fileset.toSource {
    root = ../orbit;
    fileset = fileset.unions [
      ../orbit/LAWS.bend
      ../orbit/PROOF.bend
      ../orbit/proof
      bendFiles
    ];
  };

  # The old Rust Orbit. Its suite needs host tools, as it did in Eon's package.
  oldOrbit = pkgs.rustPlatform.buildRustPackage {
    pname = "yazelix-orbit-old";
    version = "0.1.0";
    src = oldOrbitSource;
    cargoLock = workspaceCargoLock // {
      lockFile = "${oldOrbitSource}/Cargo.lock";
    };
    cargoBuildFlags = [
      "--package"
      "yazelix-orbit"
    ];
    doCheck = false;
    nativeBuildInputs = [ pkgs.zig_0_15.hook ];
    inherit (ghosttyBuildEnv) LIBGHOSTTY_VT_SYS_CPU GHOSTTY_SOURCE_DIR GHOSTTY_ZIG_SYSTEM_DIR;
    dontUseZigBuild = true;
    dontUseZigCheck = true;
    dontUseZigInstall = true;
  };

  # A worker generated from the schema plus one flag, compiled as
  # orbit/build.rs compiles the package's, for the schema-mismatch check.
  mismatchedWorker =
    pkgs.runCommandCC "yazelix-orbit-mismatched-worker"
      {
        nativeBuildInputs = [
          pkgs.bun
          pkgs.clang
          pkgs.rustc
        ];
        BEND_NO_TELEMETRY = "1";
      }
      ''
        cp -R ${workerSource}/src src
        chmod -R u+w src
        rustc --edition 2024 ${boundarySource}/tools/boundary.rs -o generate
        { cat ${boundarySource}/src/boundary.schema; echo 'flag 31 spare'; } > spare.schema
        ./generate spare.schema src
        (cd src && bun ${bend}/bend2/main.ts main.bend -o ../worker.c)
        clang -std=c11 -O2 -Wl,-z,noexecstack worker.c -lpthread -lm -o "$out"
      '';

  # A child script, orbit/tests/SCRIPT.sh, with a bash shebang.
  shellScript =
    script: pkgs.writeShellScript "orbit-${script}" (builtins.readFile (../orbit/tests + "/${script}.sh"));
  bash = "${pkgs.bash}/bin/bash";
  # Runs the real-PTY test NAME, with dashes as underscores. The child script,
  # orbit/tests/SCRIPT.sh, reaches the test in VARIABLE.
  realPtyCheck =
    {
      name,
      variable ? null,
      script ? "${name}-shell",
      timeout ? "15s",
      env ? { },
    }:
    lib.nameValuePair "orbit-${name}" (
      pkgs.runCommand "orbit-${name}"
        (
          {
            nativeBuildInputs = [ pkgs.procps ];
            ORBIT_BIN = "${orbitPackage}/bin/yazelix-orbit";
          }
          // lib.optionalAttrs (variable != null) {
            ${variable} = shellScript script;
          }
          // env
        )
        ''
          timeout ${timeout} ${orbitPackage.tests}/libexec/real-pty --ignored --exact \
            ${builtins.replaceStrings [ "-" ] [ "_" ] name} | tee test.log
          grep -q '^test result: ok. 1 passed;' test.log
          touch "$out"
        ''
    );
  realPtyChecks = builtins.listToAttrs (map realPtyCheck [
    {
      name = "lifecycle";
      variable = "PRESSURE_SHELL";
      script = "pressure-shell";
      timeout = "120s";
      env.ORBIT_BASH = bash;
    }
    {
      name = "guardian-containment";
      timeout = "120s";
      env = {
        ORBIT_BASH = bash;
        IGNORE_HUP_SHELL = shellScript "ignore-hup-shell";
        FOREGROUND_SHELL = shellScript "foreground-job-shell";
      };
    }
    { name = "delayed-output"; variable = "DELAY_SHELL"; }
    {
      name = "differential";
      variable = "DIFF_SHELL";
      timeout = "120s";
      env.OLD_ORBIT_BIN = "${oldOrbit}/bin/yazelix-orbit";
    }
    {
      name = "request-half-close";
      env.ORBIT_BASH = bash;
    }
    { name = "detached-exit"; variable = "EXIT_SHELL"; }
    { name = "three-attach"; variable = "RECONNECT_SHELL"; }
    { name = "late-observer"; variable = "LATE_OBSERVER_SHELL"; }
    { name = "detached-observer"; variable = "DETACHED_OBSERVER_SHELL"; }
    { name = "synchronized-output"; variable = "SYNC_SHELL"; }
    { name = "history"; variable = "HISTORY_SHELL"; timeout = "30s"; }
    { name = "preview"; variable = "PREVIEW_SHELL"; timeout = "30s"; }
    { name = "scroll"; variable = "SCROLL_SHELL"; timeout = "60s"; }
    { name = "return-to-live"; variable = "LIVE_SHELL"; timeout = "120s"; }
    {
      name = "schema-mismatch";
      env.MISMATCHED_WORKER = mismatchedWorker;
    }
    { name = "paste-flood"; variable = "PASTE_SHELL"; timeout = "60s"; }
    { name = "terminal-replies"; variable = "REPLY_SHELL"; timeout = "30s"; }
    {
      name = "silent-peer";
      variable = "EXIT_SHELL";
      script = "detached-exit-shell";
      timeout = "30s";
    }
    { name = "pty-reopen"; variable = "REOPEN_SHELL"; timeout = "30s"; }
    { name = "descendant-exit"; timeout = "30s"; }
    { name = "unsupported"; variable = "UNSUPPORTED_SHELL"; }
    {
      name = "interactive-pressure";
      variable = "INTERACTIVE_PRESSURE_SHELL";
      timeout = "90s";
    }
    { name = "key-live"; variable = "KEY_SHELL"; timeout = "30s"; }
    { name = "mouse"; variable = "MOUSE_SHELL"; timeout = "30s"; }
    { name = "slow-observer"; variable = "SLOW_SHELL"; timeout = "30s"; }
    {
      name = "observer-finish";
      variable = "SLOW_SHELL";
      script = "slow-observer-shell";
      timeout = "30s";
    }
    { name = "stale-socket"; variable = "STALE_SHELL"; timeout = "30s"; }
    { name = "clipboard"; variable = "CLIPBOARD_SHELL"; timeout = "30s"; }
    {
      name = "clipboard-pressure";
      variable = "CLIPBOARD_PRESSURE_SHELL";
      timeout = "60s";
    }
    { name = "command-line"; timeout = "30s"; }
    { name = "palette-reset"; timeout = "30s"; }
    { name = "selection"; variable = "SELECT_SHELL"; timeout = "30s"; }
    { name = "selection-tick"; variable = "TICK_SHELL"; timeout = "30s"; }
    { name = "refused-flush"; variable = "FLUSH_SHELL"; timeout = "60s"; }
    { name = "management-record"; timeout = "30s"; }
    { name = "management-lease"; timeout = "60s"; }
    {
      name = "terminal-identity";
      variable = "IDENTITY_SHELL";
      timeout = "30s";
      env.nativeBuildInputs = [
        pkgs.procps
        pkgs.ncurses
      ];
    }
    {
      name = "stopped-client-exit";
      variable = "STOPPED_CLIENT_SHELL";
      timeout = "60s";
    }
  ]);
in
realPtyChecks
// {
  # With only the package's terminfo, `eon` resolves and `clear` emits
  # xterm-256color's sequence, which the entry inherits; without it, `eon` is
  # unknown, so no fallback hides a missing entry (ORB-C10).
  orbit-terminfo = pkgs.runCommand "orbit-terminfo" { nativeBuildInputs = [ pkgs.ncurses ]; } ''
    export HOME="$TMPDIR"
    unset TERMINFO
    if env -u TERMINFO_DIRS infocmp -x eon; then exit 1; fi
    export TERMINFO_DIRS=${orbitPackage}/share/terminfo
    infocmp -x eon | tee infocmp.log
    grep -q '^eon|Eon terminal,$' infocmp.log
    TERM=eon tput clear > clear.out
    env -u TERMINFO_DIRS TERM=xterm-256color tput clear > expected.out
    test -s clear.out
    cmp clear.out expected.out
    touch "$out"
  '';
  # The committed boundary is what its schema generates, so a schema edit
  # without regeneration fails here.
  orbit-boundary = pkgs.runCommandCC "orbit-boundary" { nativeBuildInputs = [ pkgs.rustc ]; } ''
    rustc --edition 2024 ${boundarySource}/tools/boundary.rs -o generate
    mkdir generated
    ./generate ${boundarySource}/src/boundary.schema generated
    diff -u ${boundarySource}/src/boundary.rs generated/boundary.rs
    diff -u ${boundarySource}/src/boundary.bend generated/boundary.bend
    touch "$out"
  '';
  orbit-laws = pkgs.runCommand "orbit-laws" {
    nativeBuildInputs = [ pkgs.bun ];
    BEND_NO_TELEMETRY = "1";
  } ''
    bun ${bend}/bend2/main.ts ${lawSource}/PROOF.bend | tee proof.log
    grep -Fx 'ALL PROOFS CHECK' proof.log
    touch "$out"
  '';
}
