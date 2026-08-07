# Reference Routing

Use references to answer a named design question. Record the inspected revision,
useful constraints, and rejected approaches in the implementation bead.

## Orbit and terminal architecture

| Question | Sources | Use |
|---|---|---|
| How should a persistent terminal service separate session state from clients? | [zmx](https://github.com/neurosnap/zmx), Mitchell Hashimoto's Logimux material recorded in Orbit | Compare daemon, attachment, transport, and client boundaries. Treat Logimux as a research codename until its author publishes a product name. |
| Which terminal responsibilities can an embeddable engine own? | [libghostty](https://github.com/ghostty-org/ghostty), [rio-vt and Rio](https://github.com/raphamorim/rio), [qwertty](https://github.com/joshka/qwertty) | Compare parser, grid, scrollback, images, PTY, selection, search, serialization, and platform support. |
| Which interface belongs in a terminal test client? | [Ratatui](https://github.com/ratatui/ratatui) | Consider for diagnostics and reference clients. Do not make it the native Venus rendering foundation without a separate decision. |

Orbit owns the detailed research record and exact revisions. Eon consumes
accepted Orbit contracts instead of repeating the engine decision.

## Session workspace experience

| Question | Sources | Use |
|---|---|---|
| How should Eon organize and switch among durable terminal sessions without absorbing terminal or rendering state? | [Canario](https://rapha.land/canario/) and its [frontend at Rio `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario) | Study spaces, command-driven navigation, CWD-based filing, the global quick terminal, and on-demand pane previews. Canario's [session store](https://github.com/raphamorim/rio/blob/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario/Sources/SessionStore.swift) restarts shells and restores layout plus plain-text scrollback, so it is workspace-UX evidence rather than a durable-session model. Eon must consume exact Orbit session identities and must not simulate process survival, copy Canario source, or move PTY lifetime into the UI process. |

## Recovery experience

| Question | Source | Use |
|---|---|---|
| How should Eon help a user recover an interrupted product session without guessing or silently reopening the wrong work? | [Power Failure Resumer](https://github.com/Dicklesworthstone/power_failure_resumer) | Study interrupted-session discovery, grouping, confidence and ambiguity, previewable recovery plans, idempotent actions, verified reopen, and partial-failure reporting. Preserve child ownership and reject Ghostty-, AppleScript-, coding-agent-, or macOS-specific mechanics as Eon architecture. Do not auto-resume unknown processes or claim recovery beyond the exact Orbit and Venus contracts. |

## Composition and releases

| Question | Primary sources | Constraint to inspect |
|---|---|---|
| Can one release graph drive direct archives and package-manager outputs? | [dist documentation](https://axodotdev.github.io/cargo-dist/), [dist configuration](https://axodotdev.github.io/cargo-dist/book/reference/config.html) | Artifact matrices, installers, checksums, Homebrew support, custom build integration, and generated CI. Apply the crate and tool gate before adoption. |
| Can local execution reuse the release workflow when hosted Actions minutes, queues, or spending are constrained? | [Doodlestein Self Releaser](https://github.com/Dicklesworthstone/doodlestein_self_releaser) | Study local `act` execution, workflow compatibility limits, artifact parity, verification, and release upload boundaries. Treat it as a research reference, not a selected tool; compare local owned commands, Nix builds, hosted Actions, and other credible execution shapes. A local run does not automatically prove hosted-run parity, signing isolation, or release provenance. |
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
