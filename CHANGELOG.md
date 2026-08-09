# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Add Eon-owned live tabs and vertical pane stacks with stable-ID and direct
  directional CLI actions, deterministic JSON output, and one Orbit process per pane.
- Ship the x86_64 Linux alpha through one locked Nix flake with an Eon desktop
  entry that groups its X11/Xwayland window and uses the violet three-bend
  portal icon.
- Add `eon`, `eon run -- COMMAND...`, `eon attach`, `eon versions`, and
  `eon config-path`.
- Make bare `eon` and the desktop launcher reconnect to the initial session;
  keep `eon run` as the explicit start command.
- Supply pinned Mesa Vulkan drivers to desktop-launched Venus instead of
  depending on shell-only graphics environment variables.
- Keep Orbit sessions alive across Venus exit and reconnect them through the
  same private local socket.
- Ignore relative XDG base paths, create missing configuration and runtime
  directories with mode `0700`, preserve existing configuration-directory
  permissions, and reject unsafe existing runtime directories without mutation.
