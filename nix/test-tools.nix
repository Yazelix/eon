{ pkgs }:
let
  linux = pkgs.lib.optionals pkgs.stdenv.isLinux;
  # Rota cap fixtures copy or rename sleep, which multicall cannot support.
  coreutils = pkgs.coreutils.override { singleBinary = false; };
  # Temporary patch for cargo-nextest < 0.9.148 (nextest-rs/nextest#3624):
  # retain earlier stress iteration failures in the final RunStats under --no-fail-fast.
  cargoNextest =
    assert pkgs.lib.assertMsg (pkgs.lib.versionOlder pkgs.cargo-nextest.version "0.9.148")
      "cargo-nextest ${pkgs.cargo-nextest.version} >= 0.9.148 includes upstream #3624; remove nix/cargo-nextest-stress-exit.patch";
    pkgs.cargo-nextest.overrideAttrs (old: {
      postPatch = (old.postPatch or "") + ''
        patch -p1 < ${./cargo-nextest-stress-exit.patch}
      '';
      # Run only the fast dispatcher unit tests (including the patched stress test)
      # rather than nextest's broader suite on every Linux/macOS build.
      cargoTestFlags = [ "-p" "nextest-runner" "--lib" ];
      checkFlags = [ "runner::dispatcher::tests" ];
    });
in
{
  inherit coreutils cargoNextest;

  common = [
    # Orbit lifecycle runs bash. The Boat fixtures use Bash 4+ features.
    pkgs.bash
    # The Boat probe validates the guest's metadata JSON.
    pkgs.python3
    coreutils
    # The harness finds the worktree root and rota fixtures create repos.
    pkgs.git
  ] ++ linux [
    # eon-test isolation and rota's PID-namespace tests.
    pkgs.bubblewrap
    # flock for rota slots, setsid for Orbit lifecycle and rota run tests,
    # and unshare for rota cleanup.
    pkgs.util-linux
    # The Runtime and eon-bench fixtures each start a private D-Bus daemon.
    pkgs.dbus
  ];

  # The harness and workspace fixtures outside the Linux check's crate
  # selection need these tools. Crane adds nextest to the check itself.
  eonTestOnly = linux [
    # eon-test resolves the account and checks the sandbox's passwd entry.
    pkgs.getent
    # ps for mac-proof evidence and kill for Venus selection tests. Keep
    # procps ahead of coreutils and util-linux for its overlapping commands.
    pkgs.procps
    # ssh-keygen for the installer's signing fixtures.
    pkgs.openssh
  ] ++ [
    # The eon-test runner.
    cargoNextest
  ];
}
