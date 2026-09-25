# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

Eon is a native Wayland terminal workspace with Sessions that outlive its
window. Each tab has a launch directory and a stack of panes; `eonterm` gives
one command the same Session lifecycle.

## Demo

![Animated Eon demo: switch between Yazi and Quick Search, arrange tabs and panes, then open LazyGit and Codex](assets/demo/eon-demo.gif)

## Why Eon

- **Sessions outlive the window.** [Eon Sessions](https://github.com/Yazelix/eon-sessions)
  keeps each Session's PTY and terminal state apart from the window. Close Eon
  to detach, then `eon attach` to return to the same work.
  `eon stop GENERATION` ends Sessions.
- **Tabs remember directories.** Find a directory with Zoxide quick search or
  browse in Yazi. New panes start in that tab's directory; move tabs and stacked
  panes with the keyboard.
- **A pinned tool set.** Nix pins Anima, Nushell, Helix, Yazi, and LazyGit together.
  Git and Agent popups stay with their tab; the Agent popup uses your installed
  client.
- **Native Vulkan graphics.** [Eon Desktop](https://github.com/Yazelix/eon-desktop)
  draws terminal text and workspace chrome with `wgpu` on Vulkan. A hardware
  adapter uses the GPU; Mesa lavapipe can render in software. Eon requests blur
  for its translucent Wayland window; compositor support determines whether the
  frosted backdrop appears.

## Get started

The Nix-only alpha supports **x86_64 Linux on native Wayland**. You need Git,
Nix with flakes enabled, and a Wayland session. [Platform details](docs/USAGE.md#supported-environment)
cover the proved setup and current limits.

**Mac:** Orbit passed native Apple Silicon tests. Venus rendered and handled
input on M1, including our physical Scaleway Mac. Venus still has open Mac
checks, and Eon's Nix package targets Linux only. This release has no Mac
install path or ETA; Intel Macs are outside scope.

We're looking for Apple Silicon Mac testers. If you'd like to help,
[open an issue](https://github.com/Yazelix/eon/issues/new).

Install the tagged alpha with one command:

```sh
nix profile add --refresh github:Yazelix/eon/linux-alpha-2026-09-24
```

The tag pins the release; `eon versions` reports the component graph.

Run `eon` to start.

Fresh launches show a short, dismissible Anima animation, then ask for a
directory. Press Enter for a Zoxide match, or Tab to
browse with Yazi. Closing the window leaves Sessions alive; `eon attach` returns
to them. **Alt+/** opens the shortcut guide in the window.

To end Sessions explicitly, inspect `eon generations`, then run
`eon stop GENERATION`, `eon stop previous`, or `eon stop all`.

Linux Wayland is proved on COSMIC and private Sway. Other compositors,
non-systemd hosts, screen-reader use, and recovery after a machine restart
remain unproved. [Report a problem](https://github.com/Yazelix/eon/issues).

The current `edge` build also opens independent workspaces with **Alt+Shift+N**
or the launcher's **New Eon Window** action. `eon window new` prints an ID;
`eon windows` lists windows for reattachment or exact Stop.

## Guides

- [Using Eon](docs/USAGE.md): workspace controls, commands, EonTerm, and troubleshooting.
- [Configuration](docs/CONFIGURATION.md): terminal, shell, popups, and managed tools.
- [Development](docs/DEVELOPMENT.md): component owners, proofs, and maintainer checks.
- [Contracts](docs/CONTRACTS.md) and [architecture](docs/ARCHITECTURE.md): exact product boundaries and evidence.
- [Changelog](CHANGELOG.md): accepted user-visible changes.

## LOC scorecard

The scorecard counts tracked handwritten text and code. It excludes `.git/`,
Beads data, lock files, and generated artifacts.

| Surface | Lines |
|---|---:|
| Agent policy inputs | 269 |
| README | 105 |
| Repository ignore rules | 3 |
| License | 201 |
| Third-party notices | 37 |
| Binding license notice | 21 |
| Architecture and contracts | 2,345 |
| Distribution and references | 735 |
| User guides | 311 |
| Development guide | 42 |
| Benchmark report | 317 |
| Changelog | 381 |
| Rust source and tests | 17,888 |
| Cargo manifests | 38 |
| Component manifest | 374 |
| Nix composition | 840 |
| Product defaults | 0 |
| **Total** | **23,907** |
