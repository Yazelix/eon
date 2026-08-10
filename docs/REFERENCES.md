# Reference Routing

Use references to answer a named design question. Read only the matching route,
start with `Read first`, and inspect an additional source only when its condition
applies. Adjacent rows are not required reading. `Owner` identifies the
subsystem that may adopt the evidence; a reference does not transfer ownership.

Record the inspected revision, useful constraints, and rejected approaches in
the implementation bead. Keep historical evidence in that bead instead of this
active routing map.

## Orbit and terminal architecture

| Question or trigger | Owner | Read first | Read additionally only if | Preserve / reject |
|---|---|---|---|---|
| Does an Eon design question depend on terminal service, engine, attachment, or diagnostic-client internals? | Orbit through Eon Sessions | [Eon Sessions contracts](https://github.com/Yazelix/eon-sessions/blob/edge/docs/CONTRACTS.md) and [reference routing](https://github.com/Yazelix/eon-sessions/blob/edge/docs/REFERENCES.md) | Follow only the child route matching a demonstrated Orbit contract gap. | Consume accepted Orbit contracts. Do not repeat or reopen engine and diagnostic-client decisions in Eon. |

## Session workspace experience

| Question or trigger | Owner | Read first | Read additionally only if | Preserve / reject |
|---|---|---|---|---|
| How should Eon organize and switch among durable terminal sessions without absorbing terminal or rendering state? | Eon workspace policy and Venus presentation | [Canario](https://rapha.land/canario/) and its [frontend at Rio `3e41b8b19a1c`](https://github.com/raphamorim/rio/tree/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario) | Read its [session store](https://github.com/raphamorim/rio/blob/3e41b8b19a1cad9cd9bdfc8f7900cf61ce5a9098/frontends/canario/Sources/SessionStore.swift) only when comparing restart restoration with process survival. | Study spaces, navigation, filing, quick terminals, and previews. Consume exact Orbit identities; do not simulate survival, copy source, or move PTY lifetime into Venus. |
| How should a native client borrow durable sessions across local and SSH hosts without taking ownership of their state? | Eon lifecycle policy, Orbit sessions, and Venus presentation | [Ghosthub `0.7.0` at `072cc83559bd`](https://github.com/kenn-io/ghosthub/tree/072cc83559bd590ed5d759af05ec0a1868dbf359) | Read its [architecture](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/architecture.md), [terminal-session contract](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/terminal-sessions.md), or [threat model](https://github.com/kenn-io/ghosthub/blob/072cc83559bd590ed5d759af05ec0a1868dbf359/docs/threat-model.md) only for the corresponding ownership, reconnect, remote-helper, or security question. | Preserve backend authority, detach-only close, fresh attach, generation checks, per-host degradation, and supervised reconnect. Reject tmux, Herdr, or kwt adoption; merged inventories; activity heuristics as truth; macOS-only shape; and copied AGPL source. |

## Managed shell environment

| Question or trigger | Owner | Read first | Read additionally only if | Preserve / reject |
|---|---|---|---|---|
| How should Eon activate a consistent optional tool set after native Nushell, Bash, Zsh, and Fish configuration? | Eon managed-environment adapter; each child owns its native semantics | The affected exact upstream: [Nushell 0.113.1](https://github.com/nushell/nushell/tree/7b7df4aa68e957cf38b9d8157c35fa7523f44a6d), [Bash 5.3](https://git.savannah.gnu.org/cgit/bash.git/tag/?h=bash-5.3), [Zsh 5.9.1](https://github.com/zsh-users/zsh/tree/0e0d4ea11731c47f57bad042fbe75e3979d8a1d2), [Fish 4.7.1](https://github.com/fish-shell/fish-shell/tree/efb0223da10367031b7c887a3e40eccdf9bf7b06), [Starship 1.25.1](https://github.com/starship/starship/tree/8758daa7767d4e73874330b1e262fca66a7ffd30), [Zoxide 0.9.9](https://github.com/ajeetdsouza/zoxide/tree/9cdc6aa3740b4d8a9d62406c99e84c5de49645e9), [Atuin 18.16.1](https://github.com/atuinsh/atuin/tree/671f96b60dac49d1d2de73cc0812986a5e22ce7b), or [Carapace 1.6.3](https://github.com/carapace-sh/carapace-bin/tree/e4ed2a5ae661848b228224ad7edb20ea678d33d4). | Read [Nova startup at `f1beb34f`](https://github.com/Yazelix/nova/tree/f1beb34fe6060cfa2c0201d7f8095f6ef707f467) only when comparing cross-shell activation shape. | Reuse native startup and generated init; preserve native config and state; guard user hooks. Do not copy Nova or inherit its Mise, environment, settings, or compatibility scope. |

## Agent work orchestration

| Question or trigger | Owner | Read first | Read additionally only if | Preserve / reject |
|---|---|---|---|---|
| How should Eon integrate existing coding agents without parsing terminal output or treating a provider conversation as Eon work? | Eon provider adapter | [Agent Client Protocol at `9ef3e3800b40`](https://github.com/agentclientprotocol/agent-client-protocol/tree/9ef3e3800b4070632b54846b5ddf310fc4b35b03) | Read [initialization](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/initialization.mdx), [session setup](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/session-setup.mdx), or [prompt-turn lifecycle](https://github.com/agentclientprotocol/agent-client-protocol/blob/9ef3e3800b4070632b54846b5ddf310fc4b35b03/docs/protocol/v1/prompt-turn.mdx) only when that protocol phase is in scope. | Keep provider sessions adapter-owned and distinct from Eon work and Orbit Sessions. Reject universal-support assumptions, invented semantics, and ACP conversation state as Eon work. |
| How should Eon coordinate durable agent work without equating work, attempts, provider sessions, terminals, or processes? | Eon work and orchestration policy | [Orca orchestration guide at `8859e73980d2`](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skill-guides/orchestration.md) | Read its [state types](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/src/main/runtime/orchestration/types.ts) or [skill stub](https://github.com/stablyai/orca/blob/8859e73980d2cf81eaad465b258bbb13d54aab8e/skills/orchestration/SKILL.md) only when identity shape or release-matched guidance is in scope. | Study separate Run, Task, Dispatch, and worker identities, durable delivery, decisions, and handoff. Preserve Eon, Orbit, and Venus ownership; reject Orca product, provider, autonomy, and CLI scope. |
| How should Eon package provider-neutral operating guidance and liaison policy around existing coding agents? | Eon agent guidance and execution policy | [firstmate at `85e750ab9b76`](https://github.com/kunchenguid/firstmate/blob/85e750ab9b76df275c1f6b9e2bc95b671955bae9/README.md) | None by default. | Treat firstmate as agent-distribution evidence, not a skill or harness template. Study one liaison, visible backends, isolated attempts, and escalation; reject copied mechanisms, hidden crews, role hierarchy, and compatibility scope. |
| How should Beads-authorized work route to visible agent sessions while coordination remains distinct from planning truth? | Eon authorization and coordination policy | [NTM at `1110bce247d4`](https://github.com/Dicklesworthstone/ntm/blob/1110bce247d4bd90247301bbea8a01915159c86b/README.md) | Read [Agent Mail at `ec835d2dace0`](https://github.com/dicklesworthstone/mcp_agent_mail/blob/ec835d2dace0d89549d51bc273cb68939e2cda63/README.md) only if inboxes, acknowledgement, threads, or advisory leases are in scope. | Preserve Beads as sole work truth and Orbit as terminal owner. Reject copied tmux control, a second issue graph, automatic queue consumption, owned mail archives, and mandatory messaging infrastructure. |
| How should Venus direct human attention across many agent-bearing workspaces without becoming their source of truth? | Eon attention semantics and Venus presentation | [cmux at `a35880b1031e`](https://github.com/manaflow-ai/cmux/blob/a35880b1031e673e5e006f0ccb78c3ffa12c4bcb/README.md) | None by default. | Study compact context, unread attention, waiting reasons, feeds, notifications, and jump-to-work. Prefer structured events and exact identities; reject GUI-owned PTYs, screen or process inference, browser automation, remote control, and cmux compatibility. |
| If provider-native handoff is unavailable and a terminal fallback is explicitly approved, what safety rules should constrain it? | Eon provider-adapter policy over the Orbit input boundary | [ccmux handoff guide at `950f7a152947`](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/docs/handoff.md) | Read the [ccmux README](https://github.com/epilande/ccmux/blob/950f7a15294708f7742f62cfbe98c7278857023b/README.md) only when its command integration is being compared. | Preserve idle-only delivery, refusal of waiting agents, bounded queues, explicit targets, and visible failure. Reject screen scraping as prompt input and generic keystroke injection as an Eon agent contract. |
| How should Eon present an isolated agent attempt from launch through review without equating it with the durable work item? | Eon attempt lifecycle and Venus presentation | [Superset at `bf2e078632e2`](https://github.com/superset-sh/superset/blob/bf2e078632e260ce9e534832a03efe40960c034b/README.md) | None by default. | Study work-to-worktree, terminal, environment, diff, feedback, and explicit handoff. Keep the Bead independent; reject an Eon-owned editor, Git, dev server, browser, chat, scheduler, cloud relay, PR service, or merge policy. |
| How should Eon support cross-agent fresh-eyes review and handoff without manufacturing consensus or normalizing provider transcripts? | Eon review policy and provider adapters | [MCO at `9eff964825e4`](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/README.md) and its [invocation contract](https://github.com/mco-org/mco/blob/9eff964825e4da234d8c8079c61fb010854ae44e/docs/contracts/invocation-runtime-v1.md) | Read [Codex for Claude Code at `db52e28f4d9d`](https://github.com/openai/codex-plugin-cc/blob/db52e28f4d9ded852ab3942cea316258ae4ef346/README.md) only when provider-native persistent handoff is in scope. | Preserve raw answers, disagreement, explicit selection, and bounded execution. Reject inferred teams, consensus synthesis, vote-driven action, unbounded loops, canonicalized transcripts, and provider rescue commands as product contracts. |

## Recovery experience

| Question or trigger | Owner | Read first | Read additionally only if | Preserve / reject |
|---|---|---|---|---|
| How should Eon help a user recover an interrupted product session without guessing or silently reopening the wrong work? | Eon recovery policy over accepted Orbit and Venus contracts | [Power Failure Resumer](https://github.com/Dicklesworthstone/power_failure_resumer) | None by default. | Study discovery, ambiguity, previewable plans, idempotence, verified reopen, and partial failure. Reject product-specific mechanics, automatic unknown-process resume, and claims beyond exact child contracts. |

Composition and release references are routed by [Distribution and
Composition](DISTRIBUTION.md#reference-routing), their owning policy document.

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
