# Slice 2b: recover original local sessions and layout

**Draft for review, revised 2026-11-14.** This proposes amendments, not changes
to accepted [CONTRACTS.md](CONTRACTS.md). Yu approved visible safe degraded
recovery, original structure and safe selected state, and local same-boot
recovery without cross-version takeover. This revision incorporates Lucca's
[review](https://github.com/Yazelix/eon/pull/7#issuecomment-6768242579) and
[shared-checkpoint request](https://github.com/Yazelix/eon/pull/7#issuecomment-6838320943).
This revision also incorporates the [inline review](https://github.com/Yazelix/eon/pull/7#pullrequestreview-5896772508).
Exact text still requires Lucca's approval. Nothing here implements recovery
or authorizes starting its implementation children. All runtime acceptance
rows are **NOT RUN**.

## Verified baseline and ownership

Inspected `edge` at `c1b89205c7e4cfff77cf1cf6dd165222e7c83c7d`, EONW **v44**.
The owning epic is `eon-remote-workspaces-52i4.6`; contract approval is `.11`
under `eon-remote-workspaces-52i4`, followed by `.6.1` checkpoint, `.6.2`
restore/reconcile and `.6.3` degraded/CLI/proof. Their existing splits and
assignment rules remain authoritative. Slice 2a is landed; the `eon-hyil8`
dependency has been removed. This slice needs neither a global session manager
nor migration between windows. Checkpoint implementation and its runtime acceptance
are additionally gated on the private identity writer/retention work
`eon-t3fyd.14`, still open at this baseline. S2a identity/codec fields alone
do not satisfy that prerequisite. The owning implementation handoff must
carry this gate; this draft does not edit the upstream dependency graph.

Runtime owns **one checkpoint format and writer** for recoverable window
state. C38 owns registrations and names, not membership/layout. Orbit alone
owns terminal state, process identity, Ready publication and lifecycle
admission. A checkpoint grants no attach, signal, deletion or build-adoption
authority. C15's full-set acquisition, lifecycle lock, lease, identity,
cleanup and bounds remain mandatory. eonbase recovery is unchanged.

Current C11 Reload replaces Venus while keeping Runtime/supervisor alive.
It is not supervisor recovery or Update. Planned in-place supervisor Update
may reuse this same Runtime checkpoint topology core, not invent a second
format. Its compatibility, stale-origin admission, lease handoff, rollback
and mutation-dedup state remain owned by the Update contracts and
`eon-t3fyd.10/.11`; sharing a format does not approve those transitions.
Rune presentation, SSH and later host/disconnected chrome are separate work.

## Proposed C15 amendment: checkpoint and durable mutations

### Location, validation and logical schema

Use **`<runtime>/generations/<generation>/recovery.checkpoint`**, beside the
management records in that generation directory; `<runtime>` is the namespace
root. This is same-boot/login state, not reboot
persistence. Use the existing secure runtime-directory ownership rules,
regular no-follow files owned by the user with mode 0600, and bounded reads.
Unknown versions, duplicate IDs/references, unsafe type/owner/mode, wrong
namespace/generation or malformed structure never become partial topology.

Format version 1 uses bounded little-endian binary, matching the existing
Orbit management records and the approved private `identity` record rather
than adding a text codec. It grants no wire or process authority. The envelope
is header + length-delimited core + named sections:

- Header, in order: `format` (four ASCII bytes `EONR`), `version` (u16, 1),
  `namespace` (length-prefixed UTF-8), `generation`, `created_build`,
  `active_build` (each exactly 35 ASCII bytes, canonical `g1-` plus lowercase hex).
- Core: u32 byte length followed by the fields below in the stated order.
- Sections: u16 count, then entries `{name, required, payload}` in that order:
  length-prefixed UTF-8 name, u8 required flag (0 or 1), u32 payload byte length,
  and those payload bytes. Names are nonempty, unique and use the existing
  128-byte identity bound. An unknown required section refuses recovery and
  leaves the file untouched; an unknown optional section is ignored but retained
  byte-for-byte, with its name/flag, through every rewrite. 2b adds no sections.

All integers are little endian. The three 35-byte header identities are
fixed-width, without a length prefix. All other strings/raw paths have a u16 byte length;
arrays a u16 count; nullable values a u8 0/1 presence tag followed by the value
only for 1. Reject other tags, truncation, duplicate section names and trailing
bytes, including within the core. Lengths/counts must pass the bounds below
before allocation. The core field order is `revision` (u64), `sessions`,
`selection`, `terminal_id_high_water` (u64); nested records use this order:

| Record | Fields in encoding order |
| --- | --- |
| Session | `id`, nullable `persistent_id`, `tabs`, `selection: {active_tab: null or tN}` |
| Tab | `id`, `launch_directory_policy` (u8 0 = `fixed`, the only v1 value), `directory` (raw absolute Unix bytes), `columns`, `popups`, `selection: {selected_pane: null or pN, selected_popup: null or uN}` |
| Column | `panes`, `expanded` (pN in that nonempty column) |
| Pane | `id`, nullable `pinned_name`, nullable `tag`, `run` |
| Popup | `id`, `entry` (catalog key), `label` (saved bounded UTF-8 display label), `launch_directory` (raw absolute Unix bytes), `run` |
| Run | `kind` (u8 0 = `start-intent`, 1 = `confirmed-run`), `terminal_id`, `run_id` |

All local IDs and references (`sN`, `tN`, `pN`, `uN`), persistent IDs,
terminal IDs and run IDs encode as u16-length-prefixed UTF-8 strings, not
numeric suffixes or raw UUIDs. Local IDs use the field's exact prefix and a
canonical positive decimal N fitting u64 (no sign, zero or leading zeros),
under C15. Persistent IDs use C38's 48 lowercase hexadecimal characters.
Terminal IDs use C15's canonical `terminal-N` with positive checked u64 N.
Run IDs remain opaque nonempty UTF-8 with Orbit management's 128-byte bound;
do not invent a UUID/hex grammar. Independently validate each field before
matching identities; text equality alone is not admission. Nullable pins and
tags contain C10's already-trimmed, nonempty plain text, at most 80 Unicode
characters and 320 UTF-8 bytes, with controls/invisible format characters
refused; cleared fields are null, not empty strings. Decode rejects rather
than silently normalizing invalid saved text.

Window `selection` is `{shown_session: null or sN}`; each session retains its
active tab and each tab its selected pane and shown popup, with null when
no target is selected at that level. References stay within their parent.
`selected_pane` must be the expanded pane of its column, matching Runtime
selection/expansion state. `fixed` restores the saved directory verbatim,
including for an inert tab; it never recomputes home, caller CWD or terminal
CWD. Popup labels and directories remain saved attributes when their catalog
entry disappears, not commands or launch authority.

The following required fields describe that core and header:

| Field | Meaning |
| --- | --- |
| `format`, `version`, `revision` | `EONR`, version 1; monotonically increasing checked u64 commit revision |
| `namespace`, `generation` | Exact C15 recovery target, validated independently |
| `created_build`, `active_build` | C10 lifetime provenance; not aliases for generation, identity or recovery permission |
| `sessions` | Ordered session entries: original local `sN`, recorded confirmed persistent ID or null, ordered tabs |
| `tabs` | Original `tN`, launch-directory policy and directory bytes, ordered columns, persistent popup slots, last valid selection |
| `columns` | Ordered pane references and expansion state, with current layout invariants |
| `panes` | Original `pN`, pinned-name state and tag, exact terminal/run reference and start-intent or confirmed-run evidence |
| `popups` | Original `uN`, catalog key and saved display label/directory attributes, exact terminal/run reference and start evidence |
| `selection` | Last committed shown session, active tab, selected pane/popup references, all checked against this topology |
| `terminal_id_high_water` | Maximum terminal identity allocated, including durable intents with no remaining record |

Arrays belong to their parent; references cannot cross sessions or duplicate
a terminal/run in two slots. Paths use length-prefixed raw Unix bytes,
not base64 or lossy display strings; bounds are the existing path bounds.
Each pane/popup has a required tagged `run` union:
`{kind: "start-intent", terminal_id, run_id}` or
`{kind: "confirmed-run", terminal_id, run_id}`. Both IDs are nonempty and
valid under their canonical identity grammar. `confirmed-run` is written only
after canonical Ready confirmation for that exact pair; a saved run kind is not
current liveness evidence. A successful confirmation replaces the intent kind
for the same slot/pair in a new revision. Missing/unknown run kinds, missing IDs,
multiple discriminators, unrecognized evidence fields or duplicate run pairs
invalidate the checkpoint. Separately, the writer must reject changing a
slot's pair without an explicit structural replacement; a single decoded
snapshot cannot prove that historical transition. There is no persisted boolean `live` or third unresolved variant. Following complete
C15 acquisition, no matching record maps `start-intent` to `start-unconfirmed`
and `confirmed-run` to `ended`; a validated matching run is live regardless of
which run kind was last committed. A discovered inadmissible record still refuses
the whole set instead of being treated as absent.

Session names come from the registry, not an authoritative checkpoint copy.
Terminal provenance (`origin_build`, `record_generation`, protocols) comes
from validated records; unknown provenance stays null, never inferred from
checkpoint build fields. No terminal screen/history, input, command to replay,
attention/read acknowledgement, device-local scroll/selection or transient
picker request is stored in the core. Orbit retains its live screen and history.

Use the current protocol structural bounds: 32 sessions, 64 tabs per session,
4 columns per tab, 32 popup entries per tab and at most 256 terminal slots
**including stopped slots**, plus existing string/path limits. Cap encoded
checkpoint size at 16 MiB and validate before allocation/decoding. Creation
preflights both logical and encoded limits before side effects. Bounds are
part of format version 1, not unbounded configuration. Overflow or exhausted
revision/terminal allocation refuses further writes/creation with `capacity`,
without wrapping or stopping existing work.

Restoring saved local IDs means restoring the **same objects**, never
retargeting them. Newly created objects, extras and degraded synthetic objects
use the replacement supervisor's fresh C15 boot-clock ID range, preserving
its lock/wait and exhaustion rules. Do not replace that scheme with a saved
local-ID allocator or add its forbidden allocator file. With a valid
checkpoint the next terminal ID exceeds both its high-water and acquired
records. Decode also rejects a high-water below any saved `terminal-N`,
including stopped slots and unconfirmed intents. A damaged checkpoint cannot
supply a trusted high-water; degraded
recovery uses C15's independently acquired terminal inventory. Exact run IDs
and fresh local handles, not an untrusted numeric claim, prevent retargeting.

### Commit and effect semantics

A durable commit writes a fresh temporary file, fsyncs it, atomically renames
it over the valid checkpoint, and syncs the parent directory **before** success
is replied. Runtime serializes revisions on bounded off-control-loop I/O;
stale completion cannot overwrite newer state or republish a closed target.
Terminal I/O, Inspect, navigation and canonical Stop remain serviceable.

Two independent gates precede every new Start: the target session's exact C38
registration must be confirmed, and its new checkpoint intent must be durable.
A writable checkpoint does not satisfy registration. Pending/failed recovered
registration leaves validated existing terminals usable but refuses new Starts
with the existing registry outcome/diagnostic; it neither spawns nor substitutes
a new identity. New-session creation keeps C38's validation-before-registration
order. After confirmed registration, a Start reserves its exact slot, terminal ID and opaque run ID and
commits that intent. Only then may Runtime create the empty management record
and spawn. Orbit publishes Ready, not Runtime. Confirming the same run updates
the checkpoint; recovery never repeats Start. Failure before durable intent
has no spawn effect. Failure after a process/registry effect reports the
actual effect with `durability: "failed" | "uncertain"`, exit 1 and
`code: "checkpoint-failed"`; it never claims rollback or invites blind retry.
A lost reply remains unknown outcome under C10, exit 4.

Structural commits cover tab/pane/popup create and close, pane rename/pin/tag,
tab/pane reorder and cross-column pane moves (including resulting column
creation/removal), tab-directory changes (including C17/C18 picker completion), session
new/close/order, column expansion, and natural-exit pruning. A registry commit
followed by checkpoint failure reports that the registration exists or was
changed and that local durability failed; preserve the registration, reconcile
its exact ID on retry, never create a same-name replacement. A late picker,
registration or write completion after Stop cannot launch or republish the
removed target. Directory picks without an independently authorized Start do
not launch one.

Only selection changes may coalesce: fixed maximum 250 ms from the first
pending selection, with an explicit structural commit including the latest
valid selection. This bounded, non-configurable default avoids a new setting
and per-keystroke fsync; continuous changes cannot postpone it indefinitely.
Recovery promises the last committed selection, not the last visible click.

## Proposed C15/C33 amendment: reconcile and present

1. Acquire the complete discovered run set under C15 before publishing a
   recovered window. A discovered empty/unpublished, marked, malformed,
   mismatched or unsafe record still refuses the **whole set**. It cannot be
   ignored by calling its slot stopped. Preserve processes and diagnostic
   state; checkpoint failure grants no extra lifecycle authority.
2. Match saved slots by exact terminal **and** run ID to independently validated
   runs. Restore their original session IDs, ordering, tabs, columns, pinned
   names, tags, persistent popups and valid selection. Registry lookup failure does
   not hide live runs: retain the recorded identity internally, expose null
   persistent ID with `registration_error` until exact confirmation, and never
   substitute a registration by name/path.
3. After complete namespace enumeration, C15 record-before-spawn and
   dead-before-remove rules provide the absence evidence. A saved intent with
   no discovered record becomes `stopped`, reason `start-unconfirmed`; a
   previously confirmed run now definitively absent becomes `stopped`, reason
   `ended`. Neither is restarted. This rule does not turn any discovered
   inadmissible record into absence evidence.
4. Acquired runs not claimed by saved topology go into **one freshly registered
   Recovered session after the original sessions**. Registration failure or
   pending registration leaves its persistent ID null and visible registry
   diagnostic. Old registrations remain untouched; the new session does not
   impersonate them.
5. Restore inert pane-free tabs as `content_state: "inert"`,
   `inert_reason: "saved-without-live-target"`, not as pending pickers. Normal
   tabs use `content_state: "normal"`, `inert_reason: null`. A saved picker is
   never resumed and selecting an inert tab starts no shell. A persistent
   popup with a validated live run survives catalog removal; expose
   `catalog_state: "missing"` (otherwise `"present"`), its saved label and
   exact run. It stays usable/closable, but catalog-dependent launch/relaunch
   refuses until an explicit valid configuration exists. It is not a helper
   to kill or an implicitly replayable command.
6. Missing targets remain stopped placeholders only alongside at least one
   acquired live run. If **no live run is acquired, do not open a recovered
   window**; retain checkpoint diagnostics and report no live recovery. A
   normal fresh launch is a separate action, not a substitute recovery.
7. A sibling natural exit prunes its own live target but preserves recovered
   stopped siblings. The **last live exit still ends the local window under
   C8**; no placeholder-only window or persistence-retry owner remains.
   Explicit placeholder close removes only structural state and sends no Stop.
   No global-session or future remote empty-window policy is introduced.

Selection restoration takes no desktop focus. Malformed saved references
(missing/cross-parent IDs or a selected pane that is not its column's expanded
pane) invalidate the checkpoint; they do not enter a selection fallback.
Only a structurally valid saved selection whose target becomes unavailable
during live reconciliation chooses the first surviving structural target in
stored session/tab/column order; input remains disabled on stopped/inert targets and is never redirected to
another run merely because the old target vanished.

### Damaged, missing or over-capacity checkpoint

Checkpoint recovery depends on the private-record writer (`eon-t3fyd.14`).
A missing private record with surviving hosts retains approved C15 exact-run
recovery while allocation is build-derived and the targeted directory generation
equals the invoked build digest. Do not backfill the record or guess per-host
provenance. That preserves existing flat recovery, not checkpoint interpretation
or replacement. For checkpoint recovery, require the checkpoint's `active_build`
to equal **both** the reader's build and the independently validated private
`identity.active_build`. Any disagreement refuses with exit 1,
`checkpoint-build-mismatch`, reporting all three build identities, and leaves
the checkpoint untouched: no rename, preserve-aside, replacement or fresh
checkpoint. Header namespace must match the targeted namespace root;
generation and `created_build` must match the private record. Any readable
namespace, generation or creation-build conflict refuses with exit 1,
`checkpoint-unavailable`, identifies the conflicting field and leaves the
checkpoint untouched, with no preserve-aside, replacement or fresh checkpoint.
This no-touch rule also applies when another part of that file is corrupt.
A corrupt or ambiguous private identity refuses checkpoint recovery with exit 1,
`checkpoint-unavailable`, without guessing. A missing one takes only the flat
path above, under its stated build-derived preconditions. Once lifetimes allocate
independently of build (in-place update S2b, `eon-t3fyd.3`), crash recovery admits
the **recorded active build**, even when `created_build` differs; generation is
a lifetime, not a build digest. This grants no cross-build takeover.

An unknown version means valid `EONR` magic and a readable version other than 1;
it or an unknown required section refuses with exit 1, `checkpoint-unavailable`,
leaving the file untouched. Any other bad header is corrupt, including bad
magic or a truncated header. Only unreadable/corrupt checkpoint state of a
matching build takes the preserve-and-fresh path; if its header is unreadable,
the private record must independently establish that its active build equals
the reader's, with no readable conflicting namespace, generation or build
accepted. Missing-file and topology-plus-extra capacity cases retain the
rules below.

After matching-build admission and safe full-set C15 acquisition, checkpoint
damage, missing required core state or topology-plus-extra capacity overflow uses the same
**degraded** path: publish all acquired live runs in a synthetic Recovered
session with fresh registration and an explicit diagnostic. Do not fabricate
original grouping. Independently established legacy namespaces may retain
current flat recovery behavior; mere file absence does not prove legacy.
More than 256 live runs or any C15 acquisition violation still refuses the
whole recovery, never truncates it.

Preserve the unusable checkpoint by a no-overwrite rename to a unique sibling
`recovery.checkpoint.preserved.<nonce>`, syncing the parent. Keep it, never
delete it. Then commit a fresh checkpoint for the synthetic topology before
allowing new durable structure/Start. A missing file needs no rename. If
preservation or fresh commit fails, keep the diagnostic and write refusal;
never overwrite an unpreserved damaged file. Safe existing input, reading,
Inspect, navigation, close and canonical Stop continue. Successful preservation
and fresh commit reopen checkpoint-gated mutations, not the old topology;
new Starts additionally wait for confirmed C38 registration; the recovery diagnostic
and preserved path remain visible for this recovered window's lifetime.

A close/Stop can have a real process effect despite failed persistence. Report
exact stopped targets and durability, not a fictitious live result. Completed
canonical Stop finishes shutdown with `checkpoint_cleared: false` when clearing
failed/was uncertain (true only after durable removal). **No persistence-retry
supervisor remains.** Only unfinished process cleanup keeps C15's current
cleanup owner/lease and retry rules. A stale surviving checkpoint never starts
those stopped processes again. No one else's registration is removed.

## Proposed C10/C39 projection and outcomes

Apply the same authoritative snapshot fields to Inspect, `session ls --json`,
C39 initial/gap snapshots and live events; adapters do not invent states:

- Window `recovery: null | {state: "restored" | "degraded", reason: {code,
  detail}, preserved_path}`. Use `checkpoint-restored` for successful restore;
  degraded reason distinguishes absent, invalid, over-capacity or
  I/O failure. `detail` obeys the current bounded diagnostic limit;
  `preserved_path` is null unless preservation actually succeeded. Display paths
  use the existing CLI path convention, not a made-up raw-byte JSON encoding.
- Pane/popup `state: "live" | "stopped"`, and
  `stopped_reason: null | "ended" | "start-unconfirmed"`. All process endpoints
  are null unless independently validated live; saved endpoints confer no
  authority. Inert-tab and popup-catalog fields follow the schema above.
- `registration_error` remains registry-only. It is not overloaded with
  checkpoint, process, directory or catalog failures.
- Pre-effect write refusal: exit 1, `checkpoint-unavailable`. A refused action
  whose required stopped-slot edit was not persisted reports
  `stopped-not-persisted` and exact stopped target IDs. Post-effect persistence
  failure: exit 1, `checkpoint-failed`, actual effects and durability as above.
  Syntax remains exit 2, missing target 3, unreachable/lost reply 4. `capacity`
  is the existing bounded-resource refusal, not silent clipping.
- C39 adds `recovery-degraded` and `checkpoint-failed` with the same bounded
  diagnostic, target/effect and durability data as the snapshot/reply. Live
  event loss is repaired by initial/gap snapshots, not reconstructed history.
  Current C39 STREAM boundaries remain: recovery or supervisor re-exec starts
  a new stream even if PID/generation is unchanged. Planned `updating` is not
  terminal-ended success.

New public fields/events require the **next available EONW version after the
implementation baseline**, not reuse of v39 or an assumption v45 stays free.
The 16 MiB disk bound does not enlarge C10's 2 MiB wire-frame ceiling;
C39 retains its existing bounded/chunked snapshot framing. The checkpoint
format has its own version; equal format versions do not grant wire or process
compatibility.

## Shared checkpoint boundary and contract routing

The topology core above is the one Runtime persistence surface. Future Update
adds a named `update` section, marked required when needed to interpret its
state, without changing the v1 core. Its separately approved payload carries
transition state, pending picker, attention, recency, mutation-dedup results
and allocator ranges, and any transient popups/hidden-pane state it needs.
This draft excludes replaying input, commands,
transient UI or historical events, **not** the separately planned preservation
of mutation-dedup results across Update. Unknown required sections refuse
interpretation rather than silently dropping transition state. 2b neither
writes an Update section nor implements its consumer. Crash recovery retains
same-active-build admission as specified above; planned compatible Update must
prove its own transition and cannot bypass C15 by presenting this file.

C10's present exclusion of durable layout restoration and recovery of pane
pins and tags gains only this proposed C15 exception.

Amend C15/C33 for checkpoint/reconciliation; C8 for explicit consistency with
last-live exit, not a new empty-window exception; C10/C39 for shared projection
and outcomes; C11/C17/C18 for durable structural action and no picker replay;
C38 for exact registration reconciliation. **C42 is already Planned on edge**:
a validated checkpoint restores tab launch-directory bytes, from which C42
derives worktree usage by resolving symlinks/`..` and checking equality or
containment within the worktree. There is no stored worktree/session tie.
A flat/degraded projection that lost those directories cannot certify that
no former tab used a worktree. This is a limitation of recovered evidence,
not a new removal gate: retain C42's current visibility limits, including its
exclusion of synthetic recovered tabs and no-window behavior with
`window_checked: false`. Do not claim that degraded recovery protects the old
worktree usage or reconstruct missing launch directories from process guesses.
Valid restoration makes original tabs/directories available to C42's existing
addressed-window check; no scan of other windows or new fail-closed removal
policy is added here. Session
registration Forget is unrelated and stays outside 2b; this draft adds no
Forget policy.
No new contract ID is allocated. Existing proofs keep their recorded scope.

## Acceptance matrix

All rows are **NOT RUN**. Documentation review does not prove runtime behavior.
Checkpoint cases require the landed and validated `eon-t3fyd.14` private
identity writer; its absence only permits the explicitly limited legacy flat
path above, not a checkpoint implementation or acceptance pass.
Implementation records exact revision/component/protocol versions, isolated
installed platform, commands, artifacts and observed identities/counters.

| ID | Required observation |
| --- | --- |
| R1 | Three registered sessions, tabs/columns, pin, tag, popup and selection survive exact supervisor SIGKILL and separately exit without Stop; identical run/PID/start identities and advancing counters |
| R2 | Crashes before intent commit, before record creation, after empty record creation, after Ready and after confirmation; absence evidence vs whole-set unsafe-record refusal; no duplicate Start |
| R3 | Matching-build corrupt/unreadable/missing/wrong-owner/path checkpoint: all safely acquired runs usable; preserved original bytes/path, fresh registration and fresh writable checkpoint; failed rename/fsync stays write-refused |
| R4 | Valid topology plus extras exceeds capacity: same degraded path; too many live runs refuses whole set; no truncation, loss or extra Stop |
| R5 | Restored ended/start-unconfirmed placeholders, inert tabs and catalog-missing live popups; no picker/command replay; close placeholder sends no Stop |
| R6 | Sibling exit preserves stopped slots; final live exit ends window; no-live acquisition opens no recovered window and preserves diagnostic |
| R7 | Inject failures at file write/fsync/rename/parent-sync and registry-then-checkpoint; report real effects/durability; stale worker/picker completion cannot republish after Stop |
| R8 | Every enumerated structural action commits before success; selection coalescing bounded under continuous changes; control-loop I/O, Inspect and Stop stay serviceable |
| R9 | Full/partial canonical Stop and failed checkpoint clearing: exact stopped targets, checkpoint_cleared, retained owner only for unfinished process cleanup |
| R10 | Invalid/missing/mixed run kinds and fields, confirmation crash before/after commit, duplicate/cross-session refs, wrong generation, exhausted revision/IDs/size, replacement local-ID ranges and terminal intent high-water; no old ID retargeting |
| R11 | Registry pending/failure/retry preserves exact IDs and old registrations; extras follow originals; failed/pending registration with a successfully written fresh checkpoint still refuses Start while existing terminals remain usable; only exact registration confirmation opens Start; lost launch-directory evidence cannot certify C42 worktree non-use |
| R12 | Inspect/list, initial/gap/live C39 projection and exit codes agree; null stopped endpoints, distinct diagnostics, fresh STREAM and no event/input replay |
| R13 | Shared v1 envelope review with Update owners; unknown optional section survives a rewrite unchanged; unknown required section/version refuses without touching the file; checkpoint/reader/private-record active-build disagreement refuses without preserve-and-replace; creation A / active B once lifetimes allocate independently of build (in-place update S2b, `eon-t3fyd.3`) admits reader B and refuses reader A; independent Update transition/dedup proofs remain pending and cannot be claimed by 2b proof |
| R14 | Missing private identity retains only eligible legacy flat recovery without checkpoint read/replacement; corrupt/ambiguous private identity refuses; readable namespace/generation/creation-build conflicts, including otherwise damaged files, leave the checkpoint untouched |
| R15 | Malformed selection rejects the checkpoint; valid selection lost during reconciliation uses deterministic fallback without input redirection; malformed encodings, lengths, presence/kind tags, ID grammars and pin/tag bounds reject; unknown optional sections survive every successful rewrite byte-for-byte |

## Reference and verification notes

PR #2 is closed/superseded; its source at
`610b053c184f66144b75bcc9b73dc44f44a2561e` is reference for topology/checkpoint
ideas only, not imported implementation or transferable proof. Current edge
C10/C11/C15/C33/C38/C39/C42 and the owning beads take precedence over that
prototype and the previous draft. The [remote plan](REMOTE-WORKSPACES.md)
continues to sequence 2b before SSH attach and later takeover. No runtime
refresh, implementation test or live-session disruption is part of this
planning revision.
