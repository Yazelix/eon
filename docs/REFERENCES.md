# Reference Routing

Use references to answer a named design question. Record the inspected revision,
useful constraints, and rejected approaches in the implementation bead.

## Orbit and terminal architecture

| Question | Sources | Use |
|---|---|---|
| How should a persistent terminal service separate session state from clients? | [zmx](https://github.com/neurosnap/zmx), Mitchell Hashimoto's Logimux material recorded in Orbit | Compare daemon, attachment, transport, and client boundaries. Treat Logimux as a research codename until its author publishes a product name. |
| Which terminal responsibilities can an embeddable engine own? | [libghostty](https://github.com/ghostty-org/ghostty), [rio-vt and Rio](https://github.com/raphamorim/rio), [qwertty](https://github.com/joshka/qwertty) | Compare parser, grid, scrollback, images, PTY, selection, search, serialization, and platform support. |
| Which interface belongs in a terminal test client? | [Ratatui](https://github.com/ratatui/ratatui) | Consider for diagnostics and reference clients. Do not make it the native Venus rendering foundation without a separate decision. |

Orbit owns the detailed research record and exact revisions. Astra consumes
accepted Orbit contracts instead of repeating the engine decision.

## Composition and releases

| Question | Primary sources | Constraint to inspect |
|---|---|---|
| Can one release graph drive direct archives and package-manager outputs? | [dist documentation](https://axodotdev.github.io/cargo-dist/), [dist configuration](https://axodotdev.github.io/cargo-dist/book/reference/config.html) | Artifact matrices, installers, checksums, Homebrew support, custom build integration, and generated CI. Apply the crate and tool gate before adoption. |
| How should GitHub-hosted artifacts preserve identity? | [GitHub immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases) | Tag protection, release attestations, and asset immutability. |
| Which metadata does a macOS package need? | [Homebrew Cask Cookbook](https://docs.brew.sh/Cask-Cookbook) | Application bundles, checksums, architecture variants, quarantine, and upgrade behavior. |
| Which macOS checks must release automation prove? | [Apple notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) | Signing, hardened runtime, notarization, stapling, and Gatekeeper. |
| Can Nix create portable bundles for the chosen artifact? | [Nix `bundle` command](https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-bundle) | Experimental status, bundler constraints, closure size, and runtime portability. Do not make an experimental Nix interface the canonical distribution contract. |
| Does AppImage solve a demonstrated user need? | [AppImage documentation](https://docs.appimage.org/) | Desktop integration, update behavior, sandbox assumptions, and filesystem support. Evaluate after direct-bundle dogfood. |

## Review record template

Add this evidence to the implementing bead:

```text
Question:
Contract:
Sources and revisions:
Constraints preserved:
Options compared:
Decision:
Rejected alternatives and failure conditions:
Cheapest proof:
Exit condition:
```
