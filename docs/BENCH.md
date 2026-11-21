# Eon bench

This document defines the Bench v2 method (`eon-2r02l.11`) for paired,
leak-proof agent evaluations run through `agent-bench`
(`eon/tools/agent-bench`). It covers builder and reviewer roles (planners share
the same record store) and is separate from `eon-bench`, the terminal
throughput and memory harness in
[DEVELOPMENT.md](DEVELOPMENT.md#benchmark-harness).

Bench launch and scoring are **Linux-only** (recorded in a comment on
`eon-2r02l.11`, 2026-11-21, under Rule 12 / Platform Discipline), because
containment, transient user-scope resource caps, and host gates rely on Linux
namespaces, Bubblewrap, and systemd user scopes. Records, stream parsers, and
reports remain portable across Linux and `aarch64-darwin`.

`agent-bench` and its companion crates implement the hashed record store,
synthetic workspaces, warm task environments, fingerprint needles, and the
Linux jail today; later Bench v2 beads add the unified trial runner,
cost-beside-quality reporting, the reviewer eval rework, and the builder task
catalog:

| Surface | Exists today | Owned by later Bench v2 beads |
| --- | --- | --- |
| Records, hashes, store, diff, pooling | `eon-bench-config/1`, `eon-bench-task/1`, `eon-bench-run/1`, `eon-bench-grade/1`, `eon-bench-pooling/1`; CLI subcommands `record`, `config diff`, `pool declare`, `pool show` (`eon/tools/agent-bench`) | `eon-2r02l.14` (plan item `R1` of `eon-2r02l.11`): `eon-bench-config/2`, Bench v2 run-row fields, trial and lanes files, `agent-bench trial check` |
| Workspaces, task environments, trial runner, scoring, jail, leak checks | `workspace create`, `workspace check`, `tree-hash`, `fingerprints`, `task-env warm` (`eon/tools/agent-bench`); Linux no-network jail for Claude Code and Codex CLI (`eon/tools/agent-jail`) | Follow-up beads under `eon-2r02l` (plan items `R2`–`R8b` and `SC` of `eon-2r02l.11`), `eon-2r02l.4` (jail scorer), `eon-2r02l.7` (`jail` runner level), `eon-2r02l.8` (blind judges; `R7` is its automatic leak audit), `eon-j8iyd` (`agy` in `agent-jail`) |
| Cost-beside-quality reporting | — | Follow-up bead under `eon-2r02l` (plan item `R9` of `eon-2r02l.11`): `agent-bench report` |
| Reviewer evaluation | `review-tier eval check`, `eval prepare`, `eval score` against `eon/tools/review-tier/evals/cases.toml` | Follow-up beads under `eon-2r02l` (plan items `E1a`, `E1b`, `E2`, `E4`, `R10` of `eon-2r02l.11`): case audit/freeze, new cases, scorer rework and `eval prepare --export-diff`, reviewer role runner and reviewer case task kind |
| Builder task catalog | — | `eon-2r02l.18`: select, verify, tag, and populate [Task table](#task-table) |

In the planned Bench v2 runner (inputs and schema owned by `eon-2r02l.14`, not
present behavior), each bench trial will be driven by two inputs instead of
per-trial shell scripts:

- **Trial file (tracked TOML):** names the role (`builder` or `reviewer`), the
  arms (configuration records), the tasks or cases, the starting and maximum
  repeats (`K`), the containment level, and the pairing window.
- **Lanes file (local, never tracked):** per lane, supplies the launch template
  (`rota run ...` or a local harness wrapper), usage-read command and parser,
  quota pool floors, run cap, stagger interval, heavy-queue command, context
  handoff threshold, extra environment variable names, and the containment
  levels that lane is approved for. Account names and local paths live only in
  the lanes file so tracked files and beads stay clean, and other repositories
  can plug in their own lanes and tasks.

## The unit

The unit of evaluation is a complete **fleet configuration**, not a model name
alone. A trial compares full configurations — model, harness, effort, prompt
and rules, tools, containment level, and optional plan-review stage — on frozen
tasks taken from landed work.

### Configuration record and config hash

In `eon/tools/agent-bench/src/record.rs`, every field of a configuration record
is required, unknown fields and duplicate JSON member names are refused, and
nullable fields must still appear explicitly as `null`. There is no partial
hash. The **config hash** is SHA-256 over the RFC 8785 canonical JSON (JCS) of
the validated record; the store keeps exact canonical bytes under
`configs/<hash>.json` (or `judges/<hash>.json` when `role = "judge"`) with no
trailing newline, and reports show the 12-character lowercase hex prefix.

`eon-bench-config/1` (implemented today) pins these dimensions:

- **`bench`**: `version` (semver) and `commit` (40- or 64-hex git commit ID).
- **`role`**: `builder`, `reviewer`, `planner`, or `judge`. A `judge`
  configuration requires `rubric_sha256`; every other role refuses
  `rubric_sha256`, even as `null`.
- **`model`**: `family` and `id` (as passed to the harness, checked against the
  server-reported ID on each run).
- **`harness`**: `name`, `version`, and `binary_sha256`.
- **`effort`**: `value` (the harness-native effort value).
- **`prompt`**: `role_wrapper_sha256` (the wrapper template assembled with
  `task_text.md`), `fleet_rules` (list of `{name, sha256}` in load order,
  because load order can change behavior), and `repo_overlays` (list of
  `{path, sha256}`).
- **`harness_home`**: `manifest_sha256` (hashing settings, instructions,
  skills, plugins, MCP configuration, and hooks).
- **`permissions`**: harness-native text values for `mode`, `sandbox`, and
  `web_tools`, plus `proxy_allowlist_sha256`.
- **`limits`**: `wall_cap_multiplier` (above `0` and at most `1000`, with at
  most three decimal places) and `max_turns` (at least `1`, or `null` for no
  limit).
- **`tools`**: `rota_commit`, `rota_backend_sha256`, `rtk` (`enabled` and
  `version`, where `version` is `null` if and only if `enabled` is `false`),
  `dcg` (`version` and `config_sha256`), `heavy_sh_sha256`, and
  `jail_profile_sha256`.

`eon-2r02l.14` (planned; plan item `R1` of `eon-2r02l.11`) will bump the
configuration schema to `eon-bench-config/2`:

- Adds **`containment`** (`clone` or `jail`). The jail, proxy, and rota digests
  (`tools.jail_profile_sha256`, `permissions.proxy_allowlist_sha256`,
  `tools.rota_commit`, and `tools.rota_backend_sha256`) are required at `jail`
  and must be `null` at `clone`, so a `clone` run never fakes them. (In
  `eon-bench-config/1` today, all four fields are required non-null.)
- Adds **`plan_review`** (`none`, or the reviewer arm's config hash plus a
  round cap), so a plan-review arm's hash pins its reviewer.
- Keeps `limits.wall_cap_multiplier` on the configuration record
  (`eon/tools/agent-bench/src/record.rs`) and reuses the task record's
  `wall_cap_s` and `env.pass` / `env.exclude` rather than creating a second
  owner in the trial file.

Because `containment` and `plan_review` sit inside the hashed configuration
record, `clone` and `jail` runs or distinct plan-review setups never hash
together.

### Task record and task hash

A task record (`eon-bench-task/1`, implemented today in
`eon/tools/agent-bench/src/record.rs`) uses the same strict validation and
SHA-256 over RFC 8785 canonical JSON (`tasks/<hash>.json`). It pins:

- Identity and git commits: `repo`, `role` (`builder`, `reviewer`, or
  `planner`; never `judge`), `task_id`, `tier_introduced` (`micro`, `mini`,
  `medium`, or `large`), `base_commit`, `snapshot_object_set_sha256` (from
  `agent-bench workspace create`), `landed_commit`, and `landed_at`.
- Task text (`task_text`): `sha256` of `task_text.md` (which never holds the
  role wrapper, so wrapper changes never alter a task hash), `source_commit`,
  `cutoff`, `cutoff_evidence`, and `addendum_sha256` (SHA-256 of empty bytes
  when absent).
- Hidden answer key (`key`): `files` (`{path, sha256}`), exact `f2p` test
  names, and exact `p2p` test names, alongside `key_edits_sha256` and
  `key_author_family`.
- Environment and execution: `env` (`dev_shell`, `exclude`, `pass`,
  `env_sha256`, `lockfiles`, and `warm_target_sha256`), `test` (`command`,
  `build`, and `junit_path`), `scope_paths`, `protected_paths`, `scorer`
  (`commit` and `command`), and `wall_cap_s` (seconds, at least `1`). Among the
  timeout and environment-filter settings, only `wall_cap_s` and
  `env.pass` / `env.exclude` belong to the task record; `wall_cap_multiplier`
  stays on the configuration's `limits`.

A follow-up bead under `eon-2r02l` (plan item `R10` of `eon-2r02l.11`) will add
a task kind for reviewer cases.

### Row key, append-only rows, and pooling

Run rows (`eon-bench-run/1` in `results.jsonl`, implemented today in
`eon/tools/agent-bench/src/rows.rs`) are keyed by
**`(config hash, task hash, repeat)`**, with `repeat` counting from `1`. The
store rejects a run whose configuration role does not match the task's role (or
uses a `judge` configuration). Grade rows (`eon-bench-grade/1` in
`grades.jsonl`) are keyed by `(run key, judge config hash)` and tied to the
live run's `run_id`; grade rows require a `judge` configuration and stay for
blind judges, while reviewer `eval score` outcomes will sit directly on the
reviewer run row (planned in a follow-up bead under `eon-2r02l`, plan item
`R10` of `eon-2r02l.11`).

- **Append-only supersession:** reruns append a new row naming the replaced
  live row in `supersedes`. Superseded rows stay in the file and drop out of
  live readers; at most one row per key is live, and superseding a run retires
  grades attached to the old `run_id`.
- **Server model drift:** `Store::append_run` and `Store::append_grade` compute
  `config_drift` (`server_model_id != model.id`) rather than trusting callers,
  and loaders recompute it on read (a disagreeing flag fails loading). Drifted
  rows are kept but never pool.
- **Cross-hash pooling:** rows under two different hashes pool only through a
  pre-run `eon-bench-pooling/1` declaration (`agent-bench pool declare`,
  implemented today in `eon/tools/agent-bench/src/pooling.rs`) written before
  the new hash has any run rows. Only fields the agent does not see may change:
  `bench.commit`, `tools.heavy_sh_sha256`, `tools.jail_profile_sha256`,
  `tools.rota_commit`, and `tools.rota_backend_sha256` on configurations, or
  `scorer.commit` on tasks. Judge configurations never pool. A row under a
  declared new hash whose `started_at` predates `declared_at` is returned in
  `excluded` rather than pooled, and two pooled rows for the same repeat are an
  error.
- **Bench v2 row additions and trial check (`eon-2r02l.14`, planned):** before
  the planned runner writes the first real row, `eon-bench-run/1` will gain
  containment level, audit result, early-stop and handoff events, tokens, pool
  readings, and outcome (no schema bump needed). `agent-bench trial check` will
  expand a trial file into planned runs with full config hash, task hash, and
  repeat index and refuse on any missing field.

## Task tags, tiers and hold-outs

### Tags

Every builder task carries one **area** tag and one **size** tag:

- **Area tags:** `venus`, `runtime`, `cli`, `contracts`, `scripts` (repository
  tooling, Nix, and CI glue), `rune`.
- **Size tags** (counted by the landed change's code and test lines, excluding
  docs and beads):
  - `size-s`: up to 150 lines
  - `size-m`: 151 to 400 lines
  - `size-l`: over 400 lines

### Nested tiers

Builder tiers nest strictly: **`mini (3) ⊂ medium (6) ⊂ large (12)`**. Every
higher tier reuses every valid row from lower tiers under matching hashes (or a
valid pre-run pooling declaration).

- **`mini` (3 tasks):** says whether there is a large gap on those tasks.
- **`medium` (6 tasks):** holds two `size-l` tasks (needed to test whether an
  arm is "small beads only").
- **`large` (12 tasks):** the full 12-task builder set.

### Dated additions and hold-outs

New beads join the task set as dated additions only after landing with a hidden
fail-to-pass key that passes the task screen. Three labelled examples from the
Bench v2 plan are tagged and verified with the 12-task set in `eon-2r02l.18`:
`eon-yp6vg.62` (`rune`, `size-s`), `eon-gz0vi` (`venus`, `size-s`), and
`eon-260mh.17` (`runtime`, `size-s`). Every task records `key_author_family` so
reports track and split results by who wrote the key.

**Hold-out tasks** are never shown to a prompt-tuning session: neither their
task text nor their answer keys may enter any context used to tune prompts,
wrappers, or rules.

## Containment levels

Bench v2 defines two containment levels, **`clone`** and **`jail`**. Because
`containment` belongs to `eon-bench-config/2` (planned in `eon-2r02l.14`),
`clone` and `jail` configurations will hash differently and never pool.

### Shared run controls

The pipeline below describes the **planned** Bench v2 runner's behavior — not
present `agent-bench` behavior. It is owned by follow-up beads under
`eon-2r02l` (plan items `R2`–`R8b` and `SC` of `eon-2r02l.11`), alongside
`eon-2r02l.4` (jail scorer), `eon-2r02l.7` (`jail` runner level),
`eon-2r02l.8` (blind judges), and `eon-j8iyd` (`agy` in `agent-jail`). Both
containment levels share these steps (scheduled by the planned trial loop,
plan item `R8`), building on the workspace, fingerprint, and warm-target
primitives that exist today in `eon/tools/agent-bench`:

1. **Gates (plan item `R5`):** checks machine load and free memory; allows at
   most 2 builder runs at once and never takes a live builder slot; runs
   reviewer jobs up to the lane cap (booked with the coordinator when more than
   2 run for over an hour); enforces the lane's run cap and pool floors from
   the lanes file, starting the wall-cap clock only once the slot is held;
   staggers starts by 60 s per account; on HTTP 429, waits 10 min, retries
   once, then holds the lane and writes a notice. Held runs wait and never
   spill to another pool.
2. **Workspace, home, environment, and caps (plan item `R2`):** creates a
   single-commit synthetic workspace at `base_commit` with `.beads/` dropped
   and no remote, reflog, or hooks (`agent-bench workspace create` and
   `workspace check`, implemented today); creates a throwaway harness home with
   only the settings files named by the lane; passes an allowlisted
   environment (`PATH` without `br` and `bv`, `LANG`, `TERM`, `HOME`,
   `XDG_RUNTIME_DIR` for the user manager and heavy queue, Cargo variables, and
   lane extras; `DBUS_SESSION_BUS_ADDRESS` passes only for a lane that needs
   the desktop keyring and is recorded on the run row as a residual risk);
   applies the wall cap from `wall_cap_s` and `limits.wall_cap_multiplier`; and
   runs inside a Linux transient user scope (`MemoryMax=8G`, `CPUQuota=800%`,
   dropping to `600%` at load 24).
3. **Warm target (plan item `R6`):** reuses the warm cache produced by
   `agent-bench task-env warm` (implemented today; writes `target_sha256` in
   `warm.json`), with warm reuse matched against the task record's
   `env.warm_target_sha256`. `jail` mounts the warm target via overlay; `clone`
   creates a private copy per run after a free-disk check and removes it after
   scoring.
4. **Launch and watch (plan items `R3`, `R4`, `R8b`):** launches with the role
   wrapper plus `task_text.md` (`assembled_prompt_sha256` recorded on the row).
   Builders keep `PROGRESS.md` and finish with `READY.md`. When `plan_review`
   is set (plan item `R8b`), the run executes plan, review by the pinned
   reviewer arm, and revise up to the round cap before build, recording each
   stage's cost on the row. One event model parses each harness's stream
   (session ID, per-step context tokens, compaction steps, denied actions,
   final text). Early stop (no `READY.md`, empty final text, denied actions, or
   max turns) resumes once by session ID (`agy --conversation`, Claude Code
   `--resume`, `codex exec resume`). At the first compaction, or past the
   lane's handoff threshold (400K for `agy`) at a turn boundary, the runner
   ends the session and starts a fresh one that reads `PROGRESS.md`. Fifteen
   minutes without an event counts as a stall and stops the scope.
5. **Post-run leak audit (plan item `R7` — the automatic part of `eon-2r02l.8`'s
   leak audit; `eon-2r02l.8` keeps the blind judges):** runs a depth-3 mtime
   walk over the lane's named directories for files changed outside the
   workspace, throwaway home, and target directory, and checks every tool path
   and command in the parsed transcript against the workspace. Any access into
   another checkout, worktree, the bench store, or `cases.toml` flags the row;
   a flagged row never counts and is re-run once.
6. **Separate-copy scoring (plan item `SC` — `eon-2r02l.4`'s scorer without the
   jail; `eon-2r02l.4` keeps the jail mode on top):** overlays the hidden key
   into a separate copy, reverts and flags any edits to `protected_paths`, and
   runs exact F2P and P2P tests through the heavy queue.

### `clone` vs `jail`

| Boundary | `clone` | `jail` (`eon-2r02l.7`, `eon-j8iyd`) |
| --- | --- | --- |
| Workspace, throwaway home, env allowlist, resource scope, wall cap | Fenced | Fenced |
| Warm target | Private copy per run, removed after scoring | Overlay lower layer with empty private upper/work layers |
| Host filesystem reads and writes | **Neither read-fenced nor write-fenced by the kernel**; deterred only by the brief and `dcg` hooks, and caught after the fact by the R7 mtime walk and transcript path check | Fenced by `agent-jail` mount namespace |
| Network and web tools | **Not kernel-fenced**; relies on harness settings and hooks | Fenced by `agent-jail` unshared network namespace and host allowlist proxy; web tools off |
| Jail, proxy, and rota digests (`tools.jail_profile_sha256`, `permissions.proxy_allowlist_sha256`, `tools.rota_commit`, `tools.rota_backend_sha256`) | Must be `null` in `eon-bench-config/2` | Required non-null in `eon-bench-config/2` |
| Desktop keyring bus (`DBUS_SESSION_BUS_ADDRESS`) | Passed only for keyring-dependent lanes and recorded on the row as a residual risk | Handled by the `eon-j8iyd` keyring relay for `agy` |
| Post-run R7 leak audit, separate-copy scoring, `config_drift` | Required (audit decides whether a row counts) | Still required (jail does not replace audit, scoring, or drift checks) |

- **`clone` rules:** a lane runs at `clone` only when approved for `clone` in
  the local lanes file. At `clone`, the host filesystem is neither read-fenced
  nor write-fenced by the kernel: only the brief and the `agy` `dcg` hook (and
  for Claude Code and Codex, the fleet's normal permissions and `dcg` hooks)
  deter outside access, and the post-run leak audit (a depth-3 mtime walk plus
  a transcript path check) catches it after the fact. Lucca's approval of `agy`
  without permission prompts (`--dangerously-skip-permissions`) outside the jail
  for Gemini builds covers bench runs at `clone` (recorded in a comment on
  `eon-2r02l.11`, 2026-11-21), with the fleet's `agy` `dcg` hook required and
  the automatic post-run leak audit deciding whether a row counts.
- **`jail` rules:** runs the same pipeline inside `agent-jail`
  (`eon/tools/agent-jail`, implemented today as a Linux-only no-network jail
  for Claude Code and Codex CLI; `eon-2r02l.7` integrates it as the runner's
  `jail` level and `eon-j8iyd` adds `agy` support). `agent-jail` unshares the
  mount and network namespaces, routes only the harness through a loopback
  bridge to a host allowlist proxy, and runs every command the harness starts
  in an inner layer without network or credentials. `agent-bench fingerprints`
  (implemented today) produces answer-key needles for scanning mounted host
  paths. Headless reviewer runs at `jail` await a dedicated review sandbox.

### Reviewer containment

Today, `review-tier eval prepare <case-id> <dir>` creates a git snapshot at the
case commit (which predates its entry in
`eon/tools/review-tier/evals/cases.toml`), and
`review-tier eval score <case-id> <report-file>` scores a reviewer report.

Planned follow-up beads under `eon-2r02l` (plan items `E1a`, `E1b`, `E2`, and
`R10` of `eon-2r02l.11`) add the reviewer-role containment and scorer rework:

- `review-tier eval prepare --export-diff` (plan item `E2`) writes the case
  diff, a no-shell brief, and [REVIEW.md](../REVIEW.md) into a read-only review
  directory.
- The reviewer arm runs read-only in a snapshot at the case commit, never in a
  checkout holding `cases.toml` (plan item `R10`).
- Every reviewer transcript passes through plan item `R7`'s leak audit.
- Cases are audited, extended with new regressions, and frozen by hash
  (plan items `E1a`, `E1b`) before any arm runs, with a model family other than
  the editor reviewing the freeze. `eval score` prints the cases-file hash and
  scorer revision, supports `retired` cases without breaking IDs cited in
  [REVIEW.md](../REVIEW.md), and records `disputed` outcomes in a rulings file
  keyed by `(case, report SHA-256)`. The `eval score` result is stored directly
  on the reviewer run row.

## Settled-result rule

*Carried word for word from the Bench v2 plan (`eon-2r02l.11`, section 4). Its
"section 6" refers to [The cost table](#the-cost-table) and
[Validation](#validation) below; its "section 7" refers to the plan's follow-up
beads and cost breakdown on `eon-2r02l.11`.*

Primary outcome per builder run: **resolved** (every F2P passes, no P2P
fails). The F2P fraction is reported, never settles anything. Reviewer runs:
**caught** by the frozen scorer, disputes ruled blind by two families; when
the two disagree the case counts as not caught and is listed as split.

- **Block**: one (task, repeat) run by both arms of a comparison, same task
  hash and scorer, inside the pairing window (72 h for builders).
- **Discordant block**: exactly one arm resolved.
- **Repeats**: K = 2 on every task. A task with a discordant block or a split
  arm is **deciding** and gets 2 more (its stopping point is 4); other tasks
  stop at 2. Infra failures and audit-flagged runs are re-run, not counted.
- **Settled difference**, both of:
  1. exact two-sided sign test over discordant blocks at p ≤ 0.10: the leader
     needs w wins of n, the smallest w with 2·Σ_{i≤n−w} C(n,i)/2ⁿ ≤ 0.10:

     | n | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 |
     | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
     | w | 5 | 6 | 7 | 7 | 8 | 9 | 9 | 10 | 10 | 11 | 12 | 12 | 13 | 13 | 14 | 15 | 15 | 16 | 16 | 17 |

     Four or fewer discordant blocks never settle.
  2. **spread**: the leader's wins come from at least 2 distinct tasks at mini
     and 3 at medium or large, and the trailing arm leads (higher resolved
     rate) on no task by 0.5 or more. This stops one deterministic task,
     repeated, from carrying a verdict.
- **Not detectable**: every task at its stopping point, at least 12 blocks,
  and 2 or fewer discordant. This is weak: at 2 of 12 the one-sided 95% upper
  bound on the discordance rate is about 0.44. It licenses a cost decision
  (the cheaper arm in the binding pool), not a quality claim. Mini (at most 12
  blocks) never reaches it: with 2 or fewer discordant blocks at most two
  tasks are deciding, so mini tops out at 10 blocks. Equal-looking mini
  results end unsettled and go to medium.
- **Stop**: settled; or futility, when neither arm can reach the threshold
  even if every remaining planned block went to it; or the budget cap.
  Futility and budget stops say "unsettled at `<tier>`", and the next tier
  reuses every row. Unsettled is never reported as a win.
- **Power**: at mini a settled result needs about a 40-point gap on at least
  two tasks; medium (24 blocks, 3-task spread) sees about 30. Verdicts name
  their tier and its tasks; they do not generalize past those sizes.
- **Caveats**: repeating only deciding tasks is sequential and inflates false
  positives a little; p ≤ 0.10 is loose on purpose for a fleet decision aid,
  and the validation step (section 6) checks it.

Reviewer comparisons use the same rule with (case, repeat) as the block: K =
1, a second run on discordant cases, and a spread of at least 4 distinct
cases. Codex reviewer arms are stored reports older than 72 h; for reviewers
the window is waived when model id, config hash and case-set hash match
(models are versioned; re-running Codex costs credits), and the row says so.
Pairs use shared cases only (Codex has no reports for R33, R34, R35 or new
cases). A later Codex builder arm is paired with fresh runs of the other arm
inside 72 h; that cost is in section 7.

## The cost table

`agent-bench report` (planned in a follow-up bead under `eon-2r02l`, plan item
`R9` of `eon-2r02l.11`) will output one table per paired comparison with these
nine columns:

| Arm (config 12-hex) | Tier | Blocks | Resolved | Verdict vs pair | Wall p50 | Tokens (exact) | Pool deltas (upper bound) | Cost per resolved task, binding pool |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |

Accounting and presentation rules:

- **Tokens (exact):** parsed from each harness's event stream (plan item `R3`),
  broken down per stage (`plan`, `review`, `build`).
- **Pool deltas (upper bound):** measured from usage readings taken before and
  after each run across the relevant quota pools — Gemini weekly and five-hour
  points per account kind, the shared Claude/GPT `agy` pool, Codex credits, and
  Claude weekly and five-hour pools. **Pool deltas are upper bounds whenever
  another session shares the pool.** Each run row records the concurrent live
  runs on that pool, and `agent-bench report` reports the smallest quiet delta.
- **Cost per resolved task, binding pool:** divides binding-pool cost by
  resolved tasks so quality and capacity cost sit side by side.
- **Key-author split:** builder comparisons split results by
  `key_author_family` so keys written by one model family are visible beside
  those written by another.
- **Reviewer counts:** reviewer comparisons also count blockers and false
  positives per arm where each case's known defects allow (and an optional
  Claude-based reviewer seat run on discordant cases checks parity with an
  `agy`-hosted Claude arm under plan item `E4`).
- **Reporting hygiene:** results go to the lead and are shared with the sibling
  project's lead. Account names and local paths are never tracked, and beads
  hold no account names or quota numbers.

## Validation

Repeating only deciding tasks is sequential and mildly inflates false
positives, and `p ≤ 0.10` is intentionally loose for a fleet decision aid.
Bench verdicts are therefore checked against live fleet outcomes:

- **Builder validation:** compare each arm's bench verdict beside the live
  outcomes of real beads built by that configuration, drawn from the fleet's
  model ledgers. A bench that does not predict real beads gets fixed.
- **Reviewer validation:** beside the reviewer eval table, report blocker
  overlap per seat on live reviews from the fleet's live review ledgers where
  two seats reviewed the same change.

## Task table
