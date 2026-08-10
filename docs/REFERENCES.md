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

## Managed shell environment

| Question | Sources | Use |
|---|---|---|
| How should Eon activate a consistent optional tool set after native Nushell, Bash, Zsh, and Fish configuration? | [Nova startup at `f1beb34f`](https://github.com/Yazelix/nova/tree/f1beb34fe6060cfa2c0201d7f8095f6ef707f467), [Nushell 0.113.1](https://github.com/nushell/nushell/tree/7b7df4aa68e957cf38b9d8157c35fa7523f44a6d), [Bash 5.3](https://git.savannah.gnu.org/cgit/bash.git/tag/?h=bash-5.3), [Zsh 5.9.1](https://github.com/zsh-users/zsh/tree/0e0d4ea11731c47f57bad042fbe75e3979d8a1d2), [Fish 4.7.1](https://github.com/fish-shell/fish-shell/tree/efb0223da10367031b7c887a3e40eccdf9bf7b06), [Starship 1.25.1](https://github.com/starship/starship/tree/8758daa7767d4e73874330b1e262fca66a7ffd30), [Zoxide 0.9.9](https://github.com/ajeetdsouza/zoxide/tree/9cdc6aa3740b4d8a9d62406c99e84c5de49645e9), [Atuin 18.16.1](https://github.com/atuinsh/atuin/tree/671f96b60dac49d1d2de73cc0812986a5e22ce7b), and [Carapace 1.6.3](https://github.com/carapace-sh/carapace-bin/tree/e4ed2a5ae661848b228224ad7edb20ea678d33d4) | Reuse each shell's native startup mechanism and each tool's generated init. Keep user config and tool state in native paths, guard existing prompt, completer, and same-tool hooks, and expose exact Eon-selected binaries through the Session PATH. Use Nova as comparison evidence; do not copy its source or inherit Mise, shell-specific settings, environment mutation, or compatibility scope. |

## Agent work orchestration

| Question | Sources | Use |
|---|---|---|
| How should Eon integrate existing coding agents without parsing terminal output or treating a provider conversation as Eon work? | [Agent Client Protocol at `9ef3e3800b40`](https://github.com/agentclientprotocol/agent-client-protocol/tree/9ef3e3800b4070632b54846b5ddf310fc4b35b03), its [initialization contract](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/initialization.mdx), [session setup](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/session-setup.mdx), and [prompt-turn lifecycle](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/prompt-turn.mdx) | Study negotiated protocol versions and capabilities; explicit new, load, and resume operations; structured progress, plan, permission, tool, completion, and cancellation events; and client-provided terminal and filesystem capabilities. Keep provider sessions adapter-owned and distinct from Eon work, actions, and Orbit Sessions. Treat ACP as comparison evidence until an exact agent and platform slice proves it. Reject assuming universal ACP support, reconstructing unsupported semantics, or adopting ACP conversation state as Eon's work schema. |
| How should Eon let existing coding agents coordinate durable work without equating a work item, attempt, provider session, terminal, or process? | [Orca at `8859e73980d2`](https://github.com/stablyai/orca/tree/8859e73980d2cf81eaad465b258bbb13d54aab8e), its [orchestration guide](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skill-guides/orchestration.md), [state types](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/src/main/runtime/orchestration/types.ts), and [skill stub](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skills/orchestration/SKILL.md) | Study separate Run, Task, Dispatch, and worker-resource identities; durable inbox delivery and acknowledgement; capability-bound completion; explicit ask/reply and decision gates; full handoff versus supervised coordination; and release-matched agent guidance. Treat Orca's experimental orchestration as comparison evidence, not a selected Eon contract or dependency. Preserve Eon work and action ownership, Orbit process and terminal ownership, and Venus presentation ownership. Reject inherited Electron, mobile, editor, browser, plugin, marketplace, remote-federation, provider-integration, autonomous-scheduling, and Orca CLI compatibility scope. |
| What should an Eon agent skill provide when users already have capable coding agents? | [firstmate at `85e750ab9b76`](https://github.com/kunchenguid/firstmate/blob/85e750ab9b76df275c1f6b9e2bc95b671955bae9/README.md) | Study a portable agent distribution made from instructions, skills, policies, and state conventions; one user-facing liaison; visible interchangeable session backends; isolated attempts; explicit escalation; and distinct change-producing versus research-only task shapes. Keep provider installation, authentication, reasoning, and native session state with the provider; keep durable work in Beads and execution policy in Eon. Reject nautical role hierarchy, hidden autonomous crews, copied backend mechanisms, and firstmate compatibility as product scope. |
| How should Beads-authorized work route to visible agent sessions while coordination remains distinct from planning truth? | [NTM at `1110bce247d4`](https://github.com/Dicklesworthstone/ntm/blob/1110bce247d4bd90247301bbea8a01915159c86b/README.md) and [Agent Mail at `ec835d2dace0`](https://github.com/dicklesworthstone/mcp_agent_mail/blob/ec835d2dace0d89549d51bc273cb68939e2cda63/README.md) | Study graph-aware `br`/`bv` triage, explicit assignment gates, machine-readable control surfaces, agent identities, inboxes, acknowledgement, threads, and advisory change-intent leases. Preserve Beads as the sole work and dependency truth; messages remain coordination evidence and reservations remain advisory. Preserve Orbit terminal ownership and Eon authorization policy. Reject copied tmux control, a second issue graph, automatic queue consumption, product-owned mail archives, and mandatory Agent Mail infrastructure. |
| How should Venus direct human attention across many agent-bearing workspaces without becoming their source of truth? | [cmux at `be29dfa7c586`](https://github.com/manaflow-ai/cmux/blob/be29dfa7c5863fa97c43d7b2263bf43c70d07aa8/README.md), [ccmux at `950f7a152947`](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/README.md), and its [handoff guide](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/docs/handoff.md) | Study compact workspace context, unread attention, explicit waiting reasons, jump-to-needed-work, actionable notifications, reliable hook-based session matching, and visible handoff. Prefer structured provider events and exact Eon/Orbit identities over output heuristics. Reject screen scraping or process inference as authority, direct terminal-state ownership in Venus, inherited tmux or macOS mechanics, browser automation, remote control, and generic keystroke injection as an Eon agent contract. |
| How should Eon present an isolated agent attempt from launch through review without equating it with the durable work item? | [Superset at `bf2e078632e2`](https://github.com/superset-sh/superset/blob/bf2e078632e260ce9e534832a03efe40960c034b/README.md) and the sunsetting [Vibe Kanban at `4deb7eca8f38`](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/README.md) | Study the visible flow from chosen work to an isolated branch, worktree, terminal, environment, diff, feedback, and explicit handoff or merge decision. Treat Vibe Kanban's shutdown as lifecycle evidence against a coupled all-in-one surface. Keep the Bead independent of any attempt and require explicit ownership for worktree creation, cleanup, and integration. Reject an Eon-owned editor, Git implementation, dev server, browser, provider chat, scheduler, cloud relay, PR service, or automatic merge policy. |
| How should Eon support cross-agent fresh-eyes review and handoff without manufacturing consensus or normalizing provider transcripts? | [MCO at `9eff964825e4`](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/README.md), its [invocation contract](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/docs/contracts/invocation-runtime-v1.md), and [Codex for Claude Code at `db52e28f4d9d`](https://github.com/openai/codex-plugin-cc/blob/db52e28f4d9ded852ab3942cea316258ae4ef346/README.md) | Study explicit agent selection, preserved raw answers and disagreement, bounded execution profiles, background status/result/cancel operations, and provider-native persistent handoff. Keep review findings as evidence for the owning agent or user to judge; keep authentication, sandboxing, and conversation state with each provider. Reject inferred teams, synthetic consensus, automatic action from natural-language votes, unbounded review loops, transcript conversion as canonical Eon state, and provider-specific rescue commands as product contracts. |

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
