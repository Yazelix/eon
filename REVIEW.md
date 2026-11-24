# Reviewing Eon changes

This page is for whoever reviews a slice of Eon: a fresh agent in a converge
round, a Codex run, or a person. It says what to read, how to report, and
what failure looks like in this repository. The binding rules stay in
[`AGENTS.md`](AGENTS.md) and
[`docs/AGENT-RULES.md`](docs/AGENT-RULES.md#proof-and-review); this page
does not override them.

## Read the evidence first

1. The bead (`br show --no-db <id>`): the intended outcome and its scope.
2. The proof record. It must name the exact candidate tree. Read its
   evidence (commands, exit status, summary lines, captures) before the
   diff, and check it against the required evidence below.
3. The tier output from `review-tier classify`, as the reviewer of record
   (the lead, who starts this review) ran it from a checkout of edge; the
   author's own run does not count. Its path lines show why the slice has
   its tier; review those paths hardest.
4. The diff.

As a reviewing agent, do not build or run anything, `review-tier` included:
the tier run belongs to the reviewer of record. The author's recorded checks
and that tier output are the evidence; your job is to find what they do not
cover.

## How to report

One line per finding:

```
[blocker] path/to/file.rs:123 — what is wrong; the fix
[should-fix] path/to/file.rs:40-52 — what is wrong; the fix
[nit] docs/USAGE.md:9 — wording
```

- No file and line, no finding. A missing piece cites where it belongs: the
  line whose change requires it.
- **Blocker:** the slice would ship a defect, break a rule in AGENTS.md or
  docs/AGENT-RULES.md, or its proof does not cover the candidate tree. A
  change to `review-tier`'s rules that lowers any tier (moves a pattern or
  token down, adds to the Medium or Low list, or removes a High pattern or
  token) without Lucca's recorded approval is always a blocker.
- **Should-fix:** real, worth fixing in this slice, not dangerous.
- **Nit:** at most three per review. Nits never block.
- Name the mechanism, not the symptom: say what happens, to whom, and why
  ("the eon crate's test pins the old Venus argv, so the package tests fail"),
  not "this looks fragile".
- A problem in old code the diff does not touch gets at most one line,
  as a follow-up bead idea. It never blocks.
- End with `verdict: approve` or `verdict: changes needed`.

## What failure looks like in Eon

Each class names where it usually hides and the past cases that showed it
(the ids are eval cases in `eon/tools/review-tier/evals/cases.toml` or rows
of the eon-uxgal evidence inventory).

- **Live terminals killed** (rule 4). An error from one terminal, lease or
  tab propagating with `?` until it ends the supervisor; kill or stop paths
  that reach a running Orbit host; socket or generation changes that orphan
  running windows. Ask: does this failure end one terminal or the whole
  window? (R10, R22)
- **Contract drift** (rule 1). Behavior in a contract's area that the
  contract text, status or proof record does not match; contract text that
  changed without a recorded approval; Orbit contract records the
  governance tool rejects. (R19)
- **Wire changes without a version bump.** EONW, ORBS, ORBF or ORBM
  encodings changed in place instead of under a new version.
- **CLI parity** (rule 5). A new window action without its `eon` CLI path or
  a named bead; state a person can see that an agent cannot read as data.
- **Configurability** (rule 6). A new key, size, threshold, timing, color,
  glyph, text or default that is fixed in code without configuration and a
  documented default, and without a recorded reason in its bead.
- **Small screens** (rule 11). A visible surface without a 1280×800 capture,
  or a layout that only fits the larger size.
- **Desktop focus** (rule 9). Code that activates windows or takes focus
  where a background or non-activating surface would do.
- **Tier rules lowered.** A change to `review-tier`'s rule table that moves a
  path or token to a lower tier without Lucca's recorded approval.
- **Companion code left behind.** An interface, argument list, offset or
  routing change whose dependents were not updated: a test in another crate
  that pins the old argv, derived widths sized for the old offset, a
  discovery path that assumed the old routing. Search for the other users
  of what changed. (R02, R05, R12)
- **False success.** An early return or fallback that reports success
  without doing the work. (R11)
- **macOS portability visible in the diff.** Unix socket paths that can
  exceed 104 bytes under `/private/tmp`; temp paths compared without
  canonicalizing (`/var` is `/private/var`); goldens that include a
  platform-specific label such as a key name; accepted sockets that inherit
  `O_NONBLOCK`. (R06, R14, R16, R18)
- **Linux-only paths.** Block additions or extensions of Linux-only paths,
  dependencies or tools unless they ship with an `aarch64-darwin` alternative
  or have Lucca's recorded approval under [Platform Discipline](docs/AGENT-RULES.md#platform-discipline).
  Look for `cfg(target_os = "linux")`, Linux-only dependencies and tools,
  and Linux assumptions in shared code, scripts and tests.
- **Racy or load-bound tests.** Asserting a non-zero duration for trivial
  work; writing an executable and running it while parallel tests fork
  (`ETXTBSY`); fixed time bounds that fail under load; reading a file before
  its writer is known to have written it. (R15, R21, R27, R28)
- **Gates that cannot fail.** A check whose failure path is unreachable,
  such as `! grep` under errexit in a Nix builder. (R20)
- **Release flips outside the cut.** Version or stage changes (`0.2.0`,
  Beta) on edge before the release cut. (R01)
- **Config forward compatibility.** A key that an older running build would
  reject, making it refuse the whole file. (R23)
- **New trust boundaries.** A new local listener, socket, file mode or
  launched process: who can connect or write, and what it can make Eon do.
  (R06)
- **Compatibility shims.** Aliases, fallbacks or polish for older builds;
  Eon keeps no legacy code.

## Required evidence

Some failures only a gate catches. A missing required check blocks landing.
A missing author check is a blocker. A check the lead runs at landing is
named in your review; the landing does not proceed without its recorded
result.

| The change touches | Required check | Who |
| --- | --- | --- |
| any code, test or script | whole-workspace `nix develop -c eon-test` on the exact tree | author |
| Orbit or Venus crates | `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings` for the crate | author |
| `orbit/docs/CONTRACTS.md` or another Orbit contract record | `cargo run --locked --quiet -p orbit-governance` output (R19) | author |
| `Cargo.lock`, `nix/notices`, or new assets | `.#archive-notice-check` and `.#archive-source-check` (R08, R09) | lead at landing |
| anything in the package | `nix build` of `.#default` and the Linux checks, including the eon-tests check (R05, F14) | lead at landing |
| a new sandbox need (flock, bwrap, user namespaces) | the Nix check sandbox at landing, and a note on hosted runners (R07, F14) | lead at landing |
| any Rust code or test | `nix develop -c eon-darwin-clippy` (workspace, all targets, `aarch64-apple-darwin`, warnings denied) on the exact candidate and merged landing trees | author and lead at landing |
| a change `review-tier classify` marks `darwin: ci` or `darwin: proof`, including changes adding Linux-only paths, dependencies or tools | green candidate-branch runs for the listed `darwin-build.yml` jobs, plus `mac-proof.yml` for `proof`, before landing | lead at landing |
| a visible surface | a 1280×800 capture | author |
| an output or render path | the eon-bench gate from the Verification table | author |

The Darwin runs must cover the reviewed tree. The lead may reuse a run if
the delta from its tested tree to the reviewed tree classifies `darwin: none`.
Check the recorded tree, run IDs and results at landing, or during review
if the runs are already recorded.

## Skip or skim

- Generated protocol files: `docs/AGENT-PROTOCOLS.md`, `.agent-protocols.json`
  and the generated parts of `AGENTS.md`. Review their sources instead.
- `Cargo.lock`: check only that it matches the manifest change.
- `.beads/issues.jsonl`: read beads with `br show --no-db`.
- `licenses/`, `THIRD_PARTY_NOTICES.md`, binary assets and captures.
- Goldens and fixtures: check only that the owning test explains the
  change.

## Per tier

- **High:** also check version bumps on wire changes, contract and proof
  records, and every path the tier output lists as High.
- **Medium:** the classes above, with weight on companion code and tests.
- **Low:** docs and planning only: check that the text is accurate and that
  nothing it names went stale.
