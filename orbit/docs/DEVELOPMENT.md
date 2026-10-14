# Development and performance checks

## Local checks

```sh
cargo fmt --all --check
cargo check --locked --workspace
cargo test --locked --workspace
cargo run --locked --quiet -p orbit-governance -- .
cargo install --locked \
  --git https://github.com/luccahuguet/starcompass.git \
  --rev 95c29fa76a971726b65e1d1dc06c518d525c46a2 \
  --root target/starcompass
target/starcompass/bin/starcompass check-consumer \
  --overlay .agent-protocols.local.md \
  --exceptions .agent-protocols.exceptions.json \
  --manifest .agent-protocols.json \
  --agents AGENTS.md
cargo clippy --locked --workspace --all-targets -- -D warnings
git diff --check
br ready
```

Beads contain the dependency-ordered experiment plan.

### Orbit–Venus performance baseline

[`tools/perf/orbit-venus-baseline.sh`](../tools/perf/orbit-venus-baseline.sh)
repeats the manual Linux baseline without vendoring a benchmark corpus or
adding a product mode. It consumes exact caller-supplied Orbit and Venus
binaries. Throughput mode also consumes an exact upstream
[vtebench](https://github.com/alacritty/vtebench) checkout whose release binary
and existing `benchmarks/` corpus have already been built. Lifecycle mode uses
exact caller-supplied Neovim and Yazi binaries.

Run the local checks first, then leave the desktop idle. Benchmark runs open
native Venus windows and 50 short-lived `foot` windows, so they can change
focus and tiling temporarily. Evidence includes local paths, machine and display
details, and process logs; review it before sharing.

```sh
tools/perf/orbit-venus-baseline.sh --self-check

export ORB_BASELINE_CPUS=9-15
export ORB_BASELINE_GRID='58 93'
export ORB_BASELINE_NVIM="$(command -v nvim)"
export ORB_BASELINE_YAZI="$(command -v yazi)"

tools/perf/orbit-venus-baseline.sh throughput \
  /exact/orbit/bin/yazelix-orbit \
  /exact/venus/bin/yazelix-venus \
  /exact/vtebench-checkout \
  /tmp/eon-performance

tools/perf/orbit-venus-baseline.sh lifecycle \
  /exact/orbit/bin/yazelix-orbit \
  /exact/venus/bin/yazelix-venus \
  /tmp/eon-performance
```

The harness requires native Wayland plus `pidstat`, `taskset`, `ss`, and
standard GNU command-line tools. Throughput also requires Git; lifecycle also
requires `foot`. It validates required commands and host load before launching
Orbit, then rejects any grid mismatch before the measured workload starts.
Each mode attempt creates a unique evidence directory beneath the supplied
parent and records the exact harness, hashes, revisions, environment, process
samples, and summaries; throughput also retains raw vtebench DAT files. This
output is an internal, intentionally unstable evidence artifact rather than a
product trace format.

vtebench measures blocking PTY consumption, not frame rate or visual latency.
The lifecycle reattachment check proves process survival, transport attachment,
and authoritative grid continuity. Native Wayland does not expose another
client's keypress-to-pixel or first-visible-frame timing, so the harness reports
those visual metrics as unavailable instead of substituting title changes,
wrong-output screenshots, or internal timestamps.

## CI

The [CI workflow](../.github/workflows/ci.yml) runs on pushes to `edge` that
change Cargo metadata, Rust source or tests, `build.rs`, Rust toolchain files,
the governance tool or its governed metadata, or the workflow itself. Its
standard Ubuntu job installs checksum-verified Zig 0.15.2, logs toolchain
versions, installs Starcompass from its immutable public Git revision, and runs
the workspace and consumer-import checks above with a ten-minute limit.

`gh workflow run ci.yml --ref edge` additionally starts one manual-only Apple
Silicon macOS job. It verifies the arm64 host and toolchain, installs the
checksum-verified Zig 0.15.2 macOS archive, and runs the all-target workspace
check and full native test suite with a twenty-minute limit. Pushes do not start
the macOS job. Superseded runs are cancelled.

The repository or its owning account must retain a $0 Actions product budget
with **Stop usage when budget limit is reached**, budget threshold alerts, and
included-usage alerts enabled. The workflow does not run for unrelated
documentation.

## Governance check

The governance command checks only deterministic repository facts:

- Contract sections and crate-decision records retain their canonical labeled
  fields. Contract sections use unique `ORB-C*` IDs and a known status; proved
  sections name a full Git commit and a check or evidence. `docs/CONTRACTS.md`,
  `docs/CRATES.md`, and Beads cannot refer to an unknown contract.
- Selected crate decisions name an exact version or commit, alternatives, and
  an existing evidence Bead. Selected, planned, candidate, and deferred
  decisions remain distinct.
- Beads JSON must parse so those contract and crate-evidence links can be
  checked. `br doctor` owns tracker schema, duplicate IDs, storage, and
  integrity.

The check does not classify implementation work or interpret execution,
reference, crate, closure, portability, or protocol-exception prose;
`AGENTS.md` and applicability-aware review own that policy. It cannot establish
that an agent read a reference, that evidence is truthful, or that a runtime
contract actually passes. It deliberately does not parse `.agent-protocols.*`
or generated `AGENTS.md`: Starcompass owns that interpretation. CI installs
Starcompass from exact public Git revision
`95c29fa76a971726b65e1d1dc06c518d525c46a2` and runs its source-independent
`check-consumer` command against the four local import files. That proves their
structure, hashes, framing, and local suffix agree; it treats the canonical
protocol section as opaque. Complete canonical-protocol authentication still
uses a source checkout at the exact commit in `.agent-protocols.json`.

## Documentation map

The [contract index](CONTRACTS.md) is the durable behavioral source of
truth. The [design rationale](RATIONALE.md) records where the idea came
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

Implementation Beads begin with a recorded execution baseline, then pass the
reference gate and any triggered crate gate before code shape is chosen. Proof
is tied to exact commits, cross-repository consumers pin those revisions, and
only the user may grant a narrowly recorded protocol exception. OS-specific
runtime mechanics stay behind a narrow platform seam while the core ownership
and protocol remain platform-neutral.
