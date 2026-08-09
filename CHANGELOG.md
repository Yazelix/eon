# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Supply pinned Nushell, Starship, Zoxide, Helix, Yazi, and LazyGit defaults;
  expose them through bounded `eon-*` commands and a Session-private PATH
  without changing the user's global shell environment or importing ambient
  Helix, Yazi, or LazyGit configuration-path overrides; resolve a relative Eon
  configuration root once before passing it to child tools.
- Add Eon-owned live tabs and vertical pane stacks with stable-ID and direct
  directional CLI actions, deterministic JSON output, and one Orbit process per pane.
- Expose those actions and complete accepted workspace snapshots through the
  bounded, versioned, dependency-free EONW v1 local protocol while preserving
  human and JSON CLI projections.
- Render Eon workspace snapshots in Eon Desktop and keep reattachment available
  when the initial Session has exited but another pane remains live.
- Ship the x86_64 Linux alpha through one locked Nix flake with an Eon desktop
  entry that groups its X11/Xwayland window and uses a transparent violet
  three-fold loop icon rendered at native launcher sizes.
- Add `eon`, `eon run -- COMMAND...`, `eon attach`, `eon versions`, and
  `eon config-path`.
- Make bare `eon` and the desktop launcher reconnect to the live workspace;
  keep `eon run` as the explicit start command.
- Supply pinned Mesa Vulkan drivers to desktop-launched Venus instead of
  depending on shell-only graphics environment variables.
- Keep Orbit sessions alive across Venus exit and reconnect them through the
  same private local socket.
- Ignore relative XDG base paths, create missing configuration and runtime
  directories with mode `0700`, preserve existing configuration-directory
  permissions, and reject unsafe existing runtime directories without mutation.
