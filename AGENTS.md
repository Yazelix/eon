# Agent Guidelines

Read `docs/AGENT-PROTOCOLS.md` before planning, changing, or reviewing work.
It holds the canonical protocol text, which binds as if written here. The
source repository is needed only to update or verify the import. Do not
edit generated files directly. Edit `.agent-protocols.local.md` or
`.agent-protocols.exceptions.json`, then render from the pinned source.

## Protocol import record

- Source: `https://github.com/Yazelix/starcompass`
- Source commit: `47c20b1701b4e2eae8be853d22f6a407104791e6`
- Profiles: `greenfield`, `orchestrator`, `terminal-engine`
- Manifest: `.agent-protocols.json` (schema 1)
- Directory rules: `orbit/`, `runtime/`, `venus/`; read a directory's `AGENTS.md` before changing files under it.

| Protocol | Version | SHA-256 |
| --- | ---: | --- |
| `AP-SCOPE-001` | 4 | `8d84dc6fca6097b2055eb3ca8467565709e4c474a18c3030ae3064cc612bdfab` |
| `AP-CONTRACT-001` | 4 | `cdf2e5d69cefd34ceeeaa6b7511b21f921e2be50c17d3c01b85120b20234cff7` |
| `AP-REFERENCE-001` | 3 | `186fa74aa530c9c76ae507685461dff2b1506ecb5087230ded51f02cecb9231a` |
| `AP-MINIMAL-001` | 4 | `612bc9c62a20adf7332724d4663af0a0e591d624de2a76b1c257a9ed428db1f6` |
| `AP-DEPENDENCY-001` | 2 | `ded49add538b82de0a9c522bc8a34720f4ebbb47f39fc2f7ddb25e7aad1700d3` |
| `AP-OWNERSHIP-001` | 2 | `d218a4e0625b659ec366284110bdfce02bd66cb229ab6ba07f2317092ea13053` |
| `AP-TEST-001` | 3 | `58b5837cb679e958192b366bb15d6e34649f5a91ff9f4accdc7edb4ef5cdb873` |
| `AP-PROOF-001` | 8 | `a953744b74e4ac3b7d9ec71009939494e271395b39f7a55f371e7e4b32a909f3` |
| `AP-PLAN-001` | 12 | `5ee8deccb4b3c61ddd4a52b8ed6f69ebd30e24b65ace867871a89c75ef02a984` |
| `AP-SESSION-001` | 2 | `b901ad980f05bd52164e31b4745e87e0fa2f2e9cc0a616716670135df3d9fc82` |
| `AP-CI-001` | 3 | `c7cb65a81ce8434d02f2d19306bf93358f3624d0b77886bc91d4de93f7a779ba` |
| `AP-EXCEPTION-001` | 2 | `2c4f00299922edac83286821af07ad485e5a630bca6acbbce918d87614abb395` |
| `AP-GIT-001` | 5 | `f2c3254311de57a23a13aa382fe3285e65bb34e535ecd813d1be783ca79d4bf9` |
| `AP-ORCHESTRATOR-001` | 2 | `f78ebc0db6db525aac1d3226703af7e84cba9953c579470763831241a7d0c2ad` |
| `AP-FRONTIER-001` | 2 | `0777c74645955a26dc3829812ff26fa1a00a017c3b4cc1ac6740284f2664566c` |
| `AP-PORTABILITY-001` | 2 | `a2f5025bc7a10436b5c2f8f42c6002ccf1e361f05abbff3dea6507a3426394c0` |
| `AP-TERMINAL-001` | 2 | `c1805e12ca5c41d3ed2a6b24739fcded2ad986758fda01fdfa3132c9f8351d9c` |
| `AP-FAILURE-001` | 4 | `450e48b4e1532ff50a9a1da7faa49e3539450e0676f3c530f888d33b4b752e80` |

### Local exceptions

- `AP-TEST-001` in Agent-instruction, prompt, and skill changes in the Eon repository: The user requires the active agent to reuse its existing context instead of invoking another agent for validation.
  Approved by user on 2026-08-09; review condition: Review only if the user explicitly reauthorizes isolated agent trials.
- `AP-PROOF-001` in review loops in the Eon repository: Nits never block, and text-only fixes restart a review loop at most twice per slice.
  Approved by user on 2026-10-16; review condition: Review if a text-only slice ships a defect that another pass would have caught.

## Repository-local rules

### Eon repository rules

This repository holds Eon and the subsystems it composes, built as one Cargo
workspace:

- `orbit/`: Orbit, which owns persistent terminals, attachment, and terminal
  state.
- `venus/`: Venus, the native graphical client that renders Orbit terminals.
- `runtime/`: Eon Runtime, which owns runtime mechanisms and canonical EONW.
- `eon/` and the repository root: Eon's product crates, Nix composition,
  product documentation, and distribution.

## Status

This repository ships Eon's Nix-only x86_64 Linux Wayland beta. Every 0.x
release is beta and 1.0.0 leaves beta (`eon-versioning-beta-bfyf`). `eon-omq`
owns Eon's canonical component graph, and `eon-ax4` owns the first composed
runtime and package. The Nix-free Linux install
(`eon-qualify-nix-free-linux-public-beta-lm9p`) comes next. The approved Apple
Silicon macOS expansion (`EON-C7`, `eon-prove-full-eon-apple-silicon-macos-t8o`)
stays deferred because the user has no Mac for native proof
(`eon-next-expansion-0wfb`, 2026-10-16). When resumed, the order is Orbit
`ORB-C14`, Venus `VEN-C16`, then full Eon; macOS remains unsupported until all
three native proofs are accepted.

## Implementation Languages

Rust owns durable Eon product behavior and repository tooling. Nix owns package
resolution and composition; shell is limited to irreducible process glue.
Python may support disposable investigation but must not become checked-in Eon
policy. Add durable Python, Go, TypeScript, or another implementation language
only after an explicit user decision that names the subsystem advantage and
accepts the added toolchain and ownership cost.

## Desktop Focus

Preserve the user's active desktop focus. Prefer background, isolated, or
non-activating surfaces. Take focus only when no workable alternative exists
or alternatives give materially worse results; explain why before doing so.

## Product Boundary

Eon composes its subsystems and external tools and keeps their ownership intact:

- Orbit owns persistent terminals, attachment, and terminal state.
- Venus owns the native graphical client and renders Orbit terminals.
- Eon Runtime owns the runtime mechanisms and canonical EONW.
- Helix owns editing and language integration.
- Yazi owns file management.
- Ratconfig owns configuration editing.
- Eon owns launch policy, product configuration, component selection,
  integration contracts, updates, and distribution.

Among workspace crates, Venus and Eon Runtime depend only on the protocol crates
`orbit-protocol` and `eon-workspace-protocol`, Eon's crates depend on Eon
Runtime and the protocol crates, and only Orbit depends on `libghostty-vt`.
The protocol crates stay standalone, with no workspace-inherited settings, so
projects outside this repository can use them.

Yazelix Nova remains an independent product line. Do not make Eon a successor
repository, compatibility layer, or release channel for Nova.

Use workspace, tab, pane and terminal in product text as defined in
`docs/ARCHITECTURE.md#naming-boundary`. Technical session identities, JSON fields
and protocol names retain their spellings.

## Contract-Driven Method

Index durable contracts with their owner: `EON-C<number>` in
`docs/CONTRACTS.md`, `ORB-C<number>` in `orbit/docs/CONTRACTS.md`, and
`VEN-C<number>` in `venus/docs/CONTRACTS.md`. Never renumber or reuse an ID.
Stop for user approval before adding, changing, replacing, or retiring a
contract.

In Eon, a consequential code shape (`AP-REFERENCE-001`) is a change that adds or
changes a dependency, a protocol or wire format, a contract, or a lifecycle,
security, or trust boundary. Before such a change, record in the owning bead:

- the user-visible behavior and the one owner of each behavior and data
  structure;
- the owning `docs/REFERENCES.md` routes read, with at least one comparable
  implementation when one exists: sources and inspected revisions, constraints
  kept, rejected approaches, and the decision and its check;
- for a runtime crate, framework, bundler, installer, or release tool, also the
  version policy and exit condition. Add it only in the slice that proves its
  value. Prefer a dependency that removes a complete responsibility; avoid one
  that saves a few lines while adding a second owner for the same state.

Other changes, including presentation-only changes, need the cheapest check that
exercises them; for installed UI that is one dogfood observation. Do not use a
reference as authority outside the problem it demonstrates.

Do not mark a contract `Proven` from a document or mock. Its proof record cites
the check, the exact repository and external component revisions, the platform,
and the artifact.

## Proof and Review

- **Batch proof.** Iterate freely within a slice. Intermediate rounds produce
  candidate observations, not accepted proof. Run the installed proof once, on
  the final revision of the accepted batch.
- **One commit per change.** Run checks on the exact tree being committed and
  record the proof (check, environment, result) in the owning bead within that
  commit; the commit is the proof's revision. Do not add follow-up commits
  whose only purpose is naming the source commit. Edit a contract index only
  when a contract, its checks, status, or gap changes. Existing indexed proofs
  stay valid as written.
- **Review loop.** After each slice, run a fresh-eyes and simplification pass in
  a fresh context. Fix blocking findings (bugs, wrong claims, lost information
  or coverage, broken links or checks, scope breaches); style is optional. Stop
  when a full pass on the final revision finds nothing blocking. Code fixes
  restart the loop; text-only fixes restart it at most twice per slice. Record
  one ledger line per slice in its bead: pass count and findings.
- **Autonomous review.** Run the loop in read-only review subagents without
  waiting for the user. Each reviewer reads the owning bead itself, not a
  paraphrase, plus the diff, and reports blocking findings with `file:line`,
  then optional nits, or says that nothing is blocking. Keep briefs neutral:
  never mention earlier passes or passing checks. Exclude only gaps the bead
  records as accepted with a reason, and let a reviewer flag one anyway if it
  judges it more severe. The implementing agent fixes or records each finding
  and asks the user only when a finding needs a scope, contract, or other
  user-owned decision. Keep design decisions in the main session; delegate
  reading-heavy review, search, and investigation.

## Composition and Distribution

Use Nix as the sole published installation and composition channel until the
user approves publishing another. Keep runtime ownership distribution-neutral:
Nix may supply store paths as opaque component launch inputs, but Eon code must
not construct store paths, persist them as component identity, query the Nix
store, or invoke a Nix evaluator during normal operation.

The versioned component manifest is the one version and compatibility graph.
Every channel consumes it; none gets a second graph. Direct release bundles
remain the long-term primary adoption path. `yazelix.com` may host
documentation and installer entry points; immutable release assets, checksums,
signatures, and attestations are the artifact authority.

## Platform Discipline

Treat x86_64 Linux on native Wayland as Eon's only proved product and
distribution platform. `aarch64-darwin` is the only approved expansion and
remains Nix-only during initial proof. Additional Linux architectures, X11,
Xwayland, Intel macOS, signing, notarization, direct bundles, and other release
channels require separate scope and proof. Keep platform mechanics at the
existing Orbit, Venus, Eon runtime, and Nix composition owners; do not add a
second component graph or generic compatibility framework.

Keep Eon's Linux and Wayland contracts independent of any specific init or
service manager. Proof on systemd does not authorize a systemd runtime or
distribution requirement. Orbit owns bounded PTY process-group shutdown and
direct-child reaping without cgroup delegation; deliberately detached processes
may survive terminal stop. Claim non-systemd support only after installed
dogfood in such an environment.

Eon consumes Venus's accepted native-host boundary and does not copy
Wayland, Metal, application-lifecycle, or presentation mechanisms into the
orchestrator.

Each Orbit or Venus implementation bead records one portability disposition:
`neutral`, an isolated Linux, Wayland, or Darwin dependency, or a `platform
blocker`. A blocker names the assumption, affected contracts, replacement
shape, and removal cost, then stops for user choice before it enters a protocol
or durable format.

## Testing Discipline

Before adding a test, name the meaningful failure it detects beyond existing
coverage and the supported project harness that runs it. Follow upstream test
conventions; do not ship ignored tests or bespoke harness scaffolding merely
to accompany a patch. If no suitable harness exists, retain a runnable
reproducer and red/green evidence with the issue and report the gap in
maintained coverage.

Run the Rust test suites through `nix develop -c eon-test`, never directly
inside a live Eon session's environment: they start supervisors and helper
processes. `nix/test-sandbox.nix` owns the isolation. The command passes any
arguments to `cargo test` and otherwise runs the workspace and installer
suites. A test that needs a real display, such as Venus's ignored live
regression, runs against its own isolated display as
`venus/docs/DEVELOPMENT.md` describes.

## Local Runtime Synchronization

After any accepted runtime, package, or product-configuration change, refresh
the active `eon` Nix profile element from the current working tree before
handoff. Verify that the profile resolves to the just-built store artifact and
that the changed installed behavior or artifact passes its cheapest exact
check. A build or profile-update failure leaves the change incomplete.

Do not stop or restart a running Eon supervisor automatically because that
terminates its live terminals. Update the profile, preserve the running
processes, and report that restart is required unless the user explicitly
authorizes interruption.

## Git Workflow

Keep one linear history with this ancestry invariant:

```text
stable ⊆ main ⊆ edge
```

All tracked changes originate on `edge`. Work on `edge` by default. The `main`
and `stable` branches are promotion-only channels. Do not create merge commits
or promote without explicit user direction.

## Beads

Use `br` for all issue work. Do not edit `.beads/` files directly. Serialize
`br` write commands and run `br sync --flush-only` at the end of each session.

Use `bv --robot-triage` as the graph-aware entry point. Use only `--robot-*`
flags; bare `bv` launches an interactive interface.

A bead must name its contract, prerequisites, acceptance checks, and scope
boundary, and its reference gate when the change is consequential.

## Documentation

Update `CHANGELOG.md` only when user-visible runtime behavior, commands,
configuration, packaged tools, installation, or a proven runtime contract
changes. `orbit/CHANGELOG.md` and `venus/CHANGELOG.md` record their accepted
wire, protocol, and contract changes. Planning edits do not require changelog
entries.

## Verification

Use the narrowest route that proves every changed surface; combine routes only
when the change actually crosses them.

| Changed surface | Required proof |
| --- | --- |
| Planning, metadata, docs, or agent policy only | Beads state, changed Markdown paths/contract IDs, and diff; render/check the import only when it changes; do not refresh the runtime profile or claim runtime behavior. |
| One owned parser or logic regression | Focused red/green check plus the affected crate or package build. |
| One owned default | Cheapest consuming behavior or artifact check plus the affected build; do not add a literal mirror test. |
| Exact external component pin only | Manifest/lock identity and affected downstream build. |
| Installed runtime, UI, or product-configuration interaction | Affected profile refresh plus one isolated outermost installed dogfood observation per accepted batch. |
| Shared runtime/wire protocol, component manifest, security, runtime/build dependency, or lifecycle boundary | Full affected Rust and Nix surfaces. |
| Promotion or release | Complete delivery and promotion proof. |

Every route preserves Git and user state, validates trust boundaries, keeps
exact pins exact, preserves live terminals, and retains the cheapest check for a
known regression.

Changes to Orbit or Venus crates also keep `cargo fmt --check` and
`cargo clippy --locked --all-targets -- -D warnings` green for the changed
crate.
