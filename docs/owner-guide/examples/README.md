# Worked example: one agent, one terminal, several owners

Source: `073f03cd2b116ba885e2cffde9fef56793395740`.
This is an illustrative scenario, not captured runtime output. Read it after
chapters [4](../04-workspace-model.md) and [9](../09-security-boundaries.md).

Suppose you have a window with a project tab and an agent in one pane. Follow
the owners as you reason about an event:

| Event | Owner and consequence | Evidence |
| --- | --- | --- |
| You ask for another tab | The CLI/action mapping includes CreateTab; Runtime owns the live tabs. | [cli/parity.rs:64](../../../runtime/crates/eon-runtime/src/cli/parity.rs), [docs/ARCHITECTURE.md:323](../../ARCHITECTURE.md) |
| The agent reports blocked | Runtime's normalization maps Blocked to NeedsInput. This reports state, not task correctness. | [attention.rs:153](../../../runtime/crates/eon-runtime/src/attention.rs) |
| You detach | Orbit's lifecycle changes Attached to Detached with the same process ID. Its survival contract still requires Orbit to run. | [lifecycle.bend:23](../../../orbit/src/lifecycle.bend), [orbit/docs/CONTRACTS.md:49](../../../orbit/docs/CONTRACTS.md) |
| A supervisor disappears | The recovery contract projects exact surviving runs into a synthetic session/tab; an old registry entry alone proves no liveness. Installed Linux adoption proof remains pending. | [docs/CONTRACTS.md:3595](../../CONTRACTS.md), [docs/ARCHITECTURE.md:349](../../ARCHITECTURE.md) |
| An untrusted socket client claims a terminal ID | The caller field affects routing and validation, not authority. A client reaching the owner endpoint holds owner authority. | [docs/CONTRACTS.md:2474](../../CONTRACTS.md) |
| An installation update selects a payload | Selection applies to fresh launches; the Planned contract requires preserving running work. Its recorded live-work observation covers Nix profiles. | [docs/CONTRACTS.md:7699](../../CONTRACTS.md), [docs/CONTRACTS.md:7522](../../CONTRACTS.md), [docs/CONTRACTS.md:7854](../../CONTRACTS.md) |

Use this example to check the distinctions in your own explanation: a pane is
a view, a terminal has a live process, a registry has durable identity, and a
socket endpoint confers authority. The owning sources make those different
claims; a shared project name does not merge them
([docs/ARCHITECTURE.md:38](../../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:349](../../ARCHITECTURE.md),
[docs/CONTRACTS.md:2474](../../CONTRACTS.md)).
