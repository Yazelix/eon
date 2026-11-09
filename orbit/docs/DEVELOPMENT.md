# Development and performance checks

## Local checks

Run these from the repository root:

```sh
cargo fmt --check -p yazelix-orbit -p orbit-protocol -p orbit-governance
cargo clippy --locked -p yazelix-orbit -p orbit-protocol -p orbit-governance \
  --all-targets -- -D warnings
cargo run --locked --quiet -p orbit-governance
git diff --check
br ready
```

Cargo builds of `yazelix-orbit` run [build.rs](../build.rs), which compiles
the Bend worker with the pinned Bend compiler (`BEND_SOURCE_DIR`), bun and
the clang at `BEND_CLANG`; `nix develop` supplies all three.

`nix develop -c eon-test` runs Orbit's unit tests and its
black-box suite, [tests/lifecycle.rs](../tests/lifecycle.rs), with the rest of
the workspace in the isolated sandbox that `nix/test-sandbox.nix` defines.
`nix flake check -L` runs Orbit's Bend proofs, its boundary and terminfo checks
and every real-PTY check; [BEHAVIOR.md](BEHAVIOR.md#check) describes them and
how to run the real-PTY checks without Nix's sandbox. After a change to
[src/session.bend](../src/session.bend) or a law's automaton, regenerate the
per-fact lemmas in `proof/` with [tools/step-proof.rs](../tools/step-proof.rs),
whose header gives its usage.

Beads contain the dependency-ordered plan.

### Throughput benchmarks

From the repository root, `nix run .#bench -- orbit --out DIR` measures
release Orbit throughput detached and with a discarding client, including
frame and CPU counters. `nix run .#bench -- terminals --eon CANDIDATE/bin/eon
--source-tree STAGED_TREE_HASH --out DIR` compares Eon and Nix-pinned reference terminals with rendering
enabled on the shared isolated headless Sway launcher. The candidate is built
with an out-link; neither suite uses the live desktop or installed profile.

[Eon's development guide](../../docs/DEVELOPMENT.md#benchmark-harness)
describes the methods and `eon-bench COMMAND --help` lists their controls.
[BEHAVIOR.md](BEHAVIOR.md#throughput) preserves the earlier measurements.
The former `orbit-venus-baseline.sh` is retired: `headless` replaces its
lifecycle mode and `terminals` replaces its throughput mode. The old CPU/RSS
thresholds and desktop resize/reattachment workloads are not carried forward.
Native input-to-visible response remains `eon-bench response`.

## CI

No hosted workflow runs these checks. Orbit's former Ubuntu and manual Apple
Silicon jobs were not carried into the consolidated repository; re-homing them
is a separate `AP-CI-001` decision. The root
[Cachix workflow](../../.github/workflows/cachix.yml) builds and publishes the
Linux Nix packages only.

## Governance check

Run from the repository root, the governance command reads Orbit's contract
and crate records under `orbit/docs/` and the shared `.beads/issues.jsonl`. It
checks only deterministic repository facts:

- Contract sections and crate-decision records retain their canonical labeled
  fields. Contract sections use unique `ORB-C*` IDs and a known status; proved
  sections name a full Git commit or the known Bead whose commit is the proof,
  and a check or evidence. `CONTRACTS.md`,
  `CRATES.md`, and Beads cannot refer to an unknown contract.
- Selected crate decisions name an exact version or commit, alternatives, and
  an existing evidence Bead from the shared tracker. Selected, planned,
  candidate, and deferred decisions remain distinct.
- Beads JSON must parse so those contract and crate-evidence links can be
  checked. `br doctor` owns tracker schema, duplicate IDs, storage, and
  integrity.

The check does not classify implementation work or interpret execution,
reference, crate, closure, portability, or protocol-exception prose;
`AGENTS.md` and applicability-aware review own that policy. It cannot establish
that an agent read a reference, that evidence is truthful, or that a runtime
contract actually passes. It deliberately does not parse `.agent-protocols.*`
or generated `AGENTS.md`: Starcompass owns that interpretation.

The tool's test, run by `nix develop -c eon-test`, also enforces the workspace
dependency direction in the root [agent
rules](../../docs/AGENT-RULES.md#product-boundary), including that the protocol
crates' manifests inherit no workspace settings. It reads `cargo metadata` and
checks every kind of direct declared dependency, not code semantics or crates
reached through third-party packages. Every workspace member needs a rule.
Beyond the root rule, the test lets `eon` depend on `eon-manifest`, limits Orbit
to `orbit-protocol` and this tool to no workspace crate (chosen in
`eon-monorepo-2oko.8`), limits Eon's `mac-proof` harness to `orbit-protocol`
(`eon-mac-launch-kz4p.8`, `eon-mac-launch-kz4p.47`) and `eon-bench`, whose
`orbit` suite speaks Orbit's wire format, to it as well
(`eon-benchmarks-xl2bp.4`), and fails a new member
until the test gives it one. Orbit must keep its direct `libghostty-vt`
dependency, so renaming the engine crate fails the test instead of disabling
the rule.

## Documentation map

The [contract index](CONTRACTS.md) is the durable behavioral source of
truth. [BEHAVIOR.md](BEHAVIOR.md) states the implementation's behavior claim
by claim with the check that covers each, [HOST.md](HOST.md) the host and
worker protocol, and [BEND.md](BEND.md) the practices for writing Bend. The [design rationale](RATIONALE.md) records where the idea came
from, the ownership hypothesis, relevant prior art, tradeoffs, and explicit
graduation and stop criteria. The [reference map](REFERENCES.md) connects
existing projects and crates to the implementation slice where their evidence
is useful. The [crate decision index](CRATES.md) records which direct or
architecture-shaping dependencies are selected, rejected, or still pending.
The [Eon technology boundaries](STACK.md) record the cross-stack
language, WebAssembly, extension, and client-framework posture without
authorizing those deferred features. The [changelog](../CHANGELOG.md) records
accepted user- and consumer-visible changes without duplicating candidate
evidence or internal development work.
References are studied when a slice begins; they do not authorize additional
features or dependencies.

The [root `AGENTS.md`](../../AGENTS.md) with its
[detail](../../docs/AGENT-RULES.md) and the [Orbit `AGENTS.md`](../AGENTS.md)
own implementation policy: reference and dependency gates, proof tied to exact
commits, protocol exceptions, and the platform seam.
