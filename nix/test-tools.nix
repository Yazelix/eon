{ pkgs }:
let
  linux = pkgs.lib.optionals pkgs.stdenv.isLinux;
in
{
  common = [
    # Orbit lifecycle runs bash. The Boat fixtures use Bash 4+ features.
    pkgs.bash
    # The Boat probe validates the guest's metadata JSON.
    pkgs.python3
    # Rota cap fixtures copy or rename sleep, which multicall cannot support.
    (pkgs.coreutils.override { singleBinary = false; })
    # The harness finds the worktree root and rota fixtures create repos.
    pkgs.git
  ] ++ linux [
    # eon-test isolation and rota's PID-namespace tests.
    pkgs.bubblewrap
    # flock for rota slots, setsid for Orbit lifecycle and rota run tests,
    # and unshare for rota cleanup.
    pkgs.util-linux
  ];

  # The harness and workspace fixtures outside the Linux check's crate
  # selection need these tools. Crane adds nextest to the check itself.
  eonTestOnly = linux [
    # eon-test resolves the account and checks the sandbox's passwd entry.
    pkgs.getent
    # ps for mac-proof evidence and kill for Venus selection tests. Keep
    # procps ahead of coreutils and util-linux for its overlapping commands.
    pkgs.procps
    # dbus-daemon for eon-bench's private bus.
    pkgs.dbus
    # ssh-keygen for the installer's signing fixtures.
    pkgs.openssh
  ] ++ [
    # The eon-test runner.
    pkgs.cargo-nextest
  ];
}
