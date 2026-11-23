# 10. Packaging and proof: what does one build establish?

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 3 minutes, commands 3, code 6, evidence 5,
critic/explanation 3. Timing awaits the reader's re-check.
<!-- owner-only:end -->

## The idea

Eon's component manifest identifies selected software. External components
carry exact revisions; this repository's revision identifies in-tree components.
Nix, the build/composition system, supplies their concrete executable paths
([components/eon-v5.json:1](../../components/eon-v5.json),
[eon-manifest/src/lib.rs:271](../../eon/crates/eon-manifest/src/lib.rs),
[flake.nix:1119](../../flake.nix)).

Proof belongs to a named revision, artifact, platform and observation. A signature
also has a narrow job: authenticating checksums under a trusted key, without
establishing safe source or an uncompromised build
([docs/CONTRACTS.md:22](../CONTRACTS.md),
[docs/DISTRIBUTION.md:78](../DISTRIBUTION.md)).

## Run it

From the repository root, in Nushell. Read composition and one historical proof:

```nu
^nl -ba flake.nix | ^sed -n '1119,1149p'
^nl -ba docs/CONTRACTS.md | ^sed -n '28,41p'
```

## What to look at

Find the Orbit/Venus paths and the wrapper environment. In the proof record,
identify the revision, Linux platform and private Sway observation; lines 38–41
explain why this old proof does not qualify today's build
([flake.nix:1121](../../flake.nix), [docs/CONTRACTS.md:28](../CONTRACTS.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [flake.nix:1119-1149](../../flake.nix):

- Lines 1119–1124 bind selected components to executable paths.
- Lines 1125–1135 add helper/shell paths and terminfo, the terminal-capability
  data programs consult.
- Lines 1136–1144 merge component, tool and output-relative settings. `//`
  merges records; `${...}` inserts a Nix value.
- Lines 1145–1149 give the environment and executable to the launch wrapper.

The chain starts with manifest selection at [flake.nix:70](../../flake.nix)
and [flake.nix:90](../../flake.nix). The product build recipe starts at
[flake.nix:1154](../../flake.nix), with full-product composition at
[flake.nix:1243](../../flake.nix). Reading this chain establishes no build result.

</details>
<!-- under-the-hood:end -->

## Read the evidence

| Evidence | What it establishes |
| --- | --- |
| Contract | obligation and scope |
| Source inspection | mechanism at a revision |
| Automated check | cases exercised by a run |
| Installed proof | observed artifact/platform/interaction |
| Independent review | scrutiny of the change and evidence |

Choose the Verification route for the changed surface. Reading a test supplies
source evidence; running it and observing an installed interaction supply
different evidence ([docs/AGENT-RULES.md:75](../AGENT-RULES.md)).

The record at [docs/CONTRACTS.md:28](../CONTRACTS.md) names revision
`795d735a8f61ef439c56d68e1cc4d3929af0dfcf`, automated checks on x86_64 Linux
and specific installed artifacts observed through private Sway. Lines 38–41
limit it to historical candidate evidence after the Bend import. Review cannot
replace a missing observation ([REVIEW.md:111](../../REVIEW.md)).

<!-- owner-only:start -->
**Owner only: review workflow.** A **risk tier** is the review tool's
classification of a change. High uses fresh Opus and a second-lab reviewer.
Medium uses fresh Codex and Neon at high effort, with at least one reviewer
outside the builder's lab. Low gets one second-lab review. Proof and Review
lists the seat and fallback rules.
**Converge** names the review, fix-or-rebut and required finder-re-check loop.
The ledger records findings
and candidate trees; missing required author evidence blocks landing
([Proof and Review](../AGENT-RULES.md#proof-and-review),
[REVIEW.md:111](../../REVIEW.md)).
<!-- owner-only:end -->

## The hardest question

> “What is reproducible, supported and proved, and who independently checked it?”

Name the build and scoped observation. The source stage is Alpha; beta needs
release-owner authorization. Linux Wayland is supported and Apple Silicon
macOS is a Nix-only preview
([product.rs:8](../../eon/crates/eon/src/product.rs),
[docs/DISTRIBUTION.md:47](../DISTRIBUTION.md), [SECURITY.md:9](../../SECURITY.md)).

Hosted/local bit-for-bit parity remains unproven. The historical record does
not name its independent reviewers; request the review ledger for the change
you are assessing. A policy requiring review is not evidence that it happened
([docs/INSTALLER.md:198](../INSTALLER.md),
[docs/CONTRACTS.md:28-41](../CONTRACTS.md),
[Proof and Review](../AGENT-RULES.md#proof-and-review)).

<!-- owner-only:start -->
## Explain it back (owner only)

Name the old proof's revision, platform and result. Explain why it does not
qualify this checkout, and what you would ask its reviewers for.
<!-- owner-only:end -->

## Going deeper

- [Verification and review, docs/AGENT-RULES.md:75](../AGENT-RULES.md).
- [Release trust, docs/DISTRIBUTION.md:35](../DISTRIBUTION.md).
- Finish with the [answers sheet](answers.md).
