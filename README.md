# Eon

*Build for Eons.*

![Glowing violet three-fold Eon loop](assets/eon.png)

Eon is a native Wayland workspace for long-running work: coding agents, builds,
servers and editors. Its **terminals keep running after you close the window**.
Run `eon attach` and every tab, pane and terminal is where you left it.

## Demo

![Animated Eon demo: switch between Yazi and Quick Search, arrange tabs and panes, then open LazyGit and Codex](assets/demo/eon-demo.gif)

## Why Eon

- **Close the window, keep the work.** [Orbit](orbit/README.md) runs each
  terminal's process and state apart from the window. Closing Eon detaches;
  `eon attach` returns to the same terminals, and `eon stop` ends them when you
  choose.
- **Made for coding agents.** Each tab has an Agent popup (**Alt+Shift+L**) that
  starts the first installed of Codex, Grok, OpenCode, Pi or Claude and keeps
  its session while hidden. When you're signed in, Codex and Claude quota appear
  in the top bar.
- **A tab per project.** Each tab starts in a directory you pick with Zoxide or
  browse to in Yazi. New panes open there, and the tab's Git popup runs LazyGit
  in it.
- **One pinned tool set.** Nushell, Helix, Yazi, LazyGit and Anima are pinned
  together, so every install gets the same tools.
- **One terminal emulator, natively drawn.** Unlike tmux inside another
  terminal, one emulator owns each terminal's state, and
  [Venus](venus/README.md) draws it with `wgpu` on Vulkan.

## Get started

You need **x86_64 Linux on native Wayland**, Git, and Nix with flakes enabled.
Install the latest release, 0.1.1:

```sh
nix profile add --refresh github:Yazelix/eon/v0.1.1
```

Run `eon`. After a short animation (most keys skip it), pick a directory: press
Enter for a Zoxide match or Tab to browse with Yazi. A terminal opens there.

| Shortcut | Action |
|---|---|
| Alt+/ | Show every shortcut |
| Alt+Shift+T | New tab |
| Alt+M | New pane in this tab |
| Alt+H / Alt+L | Previous or next tab |
| Alt+K / Alt+J | Previous or next pane |
| Alt+Shift+L | Agent popup |
| Alt+Shift+J | Git popup |

Close the window whenever you like. To come back or end work:

```sh
eon attach               # return to your terminals
eon generations          # list running and older generations
eon stop GENERATION      # end one generation's terminals
```

`eon --help` lists every command. `eon versions` reports the pinned component
graph.

An optional, third-party [binary cache](docs/DISTRIBUTION.md#optional-binary-cache)
can avoid local builds for the `edge` builds it holds; tags have no guaranteed
coverage.

## Status and limits

Eon is in beta: commands, configuration and behavior may still change between
0.x releases, and 1.0.0 marks the end of beta. It installs only through Nix for
now. It was proved on COSMIC and private Sway before the move to one
repository. Here, installed checks so far run on a private headless Sway, plus
a brief COSMIC launch check; the full desktop proofs and the bounded Orca
[reader qualification](docs/CONTRACTS.md#eon-c8--durable-tab-and-pane-workspace)
await re-proof. Other compositors, non-systemd hosts and proprietary NVIDIA
drivers are unproved. Restarting the machine ends live terminals.

**Mac:** Orbit passed native Apple Silicon tests, and Venus rendered and handled
input on M1, before the move to one repository; both await re-proof here. Eon has
no Mac install path or ETA yet, and Intel Macs are out of scope. We're looking for
Apple Silicon testers: [open an issue](https://github.com/Yazelix/eon/issues/new).

[Report a problem](https://github.com/Yazelix/eon/issues).

## Guides

- [Using Eon](docs/USAGE.md): workspace controls, commands, Anima, EonTerm and troubleshooting.
- [Configuration](docs/CONFIGURATION.md): terminal, shell, popups, agent quota and managed tools.
- [Distribution](docs/DISTRIBUTION.md): Nix composition, the binary cache and release policy.
- [Manual Linux bundle](docs/MANUAL-BUNDLE.md) and [Linux installer](docs/INSTALLER.md): unpublished Nix-free candidates.
- [Changelog](CHANGELOG.md): accepted user-visible changes.

## Contributing

Contributors and coding agents start with [AGENTS.md](AGENTS.md) and
[Development](docs/DEVELOPMENT.md). [Architecture](docs/ARCHITECTURE.md) and
[contracts](docs/CONTRACTS.md) record component ownership, exact product
boundaries and proof.
