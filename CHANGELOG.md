# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Eon is now in beta, versioned by number. Every 0.x release is beta, and
  1.0.0 marks the end of beta. Releases will bump Eon's version and carry
  `vX.Y.Z` tags instead of date tags; `linux-alpha-2026-10-14` was version
  0.1.0. Installing without Nix is a later 0.x milestone.

## linux-alpha-2026-10-14

The first tag of this repository. It ships everything listed below and in the
summary that follows.

- Build Orbit, Venus and Eon Runtime from this repository. `eon versions`
  reports `workspace` as their revision; the generation ID on its first line
  identifies the build. Orbit's and Venus's licenses install under
  `share/licenses/orbit/` and `share/licenses/venus/`.

- Retire EON-C22 and EON-C26: the `codex app-server` collector and the Claude
  status-line collector no longer exist, and EON-C27 owns quota collection
  through CodexBar.

## Before this repository

Eon was developed across four repositories until 2026-10-14; their history is
kept in a private archive. In summary:

- The `linux-alpha-2026-09-24` tag shipped the Nix-only x86_64 Linux Wayland
  alpha: terminals that outlive the window, `eon attach`, generations and
  `eon stop`, tabs with launch directories and pane stacks, the Zoxide and Yazi
  directory picker, Git and Agent popups, EonTerm, pinned managed shells
  and tools, configurable typography, opacity and compositor blur, and the
  native Eon Bar, shortcut viewer and Codex quota chip.
- Later `edge` builds added the Anima welcome, `eon anima` and its popup,
  independent windows (Alt+Shift+N and `eon window`),
  bottom shortcut hints, the `EON <version>` label, later tabs starting in the
  home directory, Codex and Claude quota through pinned CodexBar CLI over EONW v9,
  grouped CLI help, the four-click selection ladder, Accelerando glyphs for
  `eon anima accelerando`, screen-reader caret and text geometry, the optional
  `eon-yazelix` binary cache, a pinned `nix develop` environment, and an
  unpublished manual Linux bundle and installer.
