# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Ship the x86_64 Linux alpha through one locked Nix flake with an Eon desktop
  entry that groups its X11/Xwayland window and uses the violet Möbius icon.
- Add `eon`, `eon run -- COMMAND...`, `eon attach`, `eon versions`, and
  `eon config-path`.
- Keep Orbit sessions alive across Venus exit and reconnect them through the
  same private local socket.
- Ignore relative XDG base paths, create missing configuration and runtime
  directories with mode `0700`, preserve existing configuration-directory
  permissions, and reject unsafe existing runtime directories without mutation.
