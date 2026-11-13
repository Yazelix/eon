# Slice 2b: recover original local sessions and layout

**Draft for review, revised 2026-11-13.** This proposes amendments, not changes
to accepted [CONTRACTS.md](CONTRACTS.md). Yu approved visible safe degraded
recovery, original structure and safe selected state, and local same-boot
recovery without cross-version takeover. This revision incorporates Lucca's
[review](https://github.com/Yazelix/eon/pull/7#issuecomment-6768242579) and
[shared-checkpoint request](https://github.com/Yazelix/eon/pull/7#issuecomment-6838320943).
Exact text still requires Lucca's approval. Nothing here implements recovery
or authorizes starting its implementation children. All runtime acceptance
rows are **NOT RUN**.

## Verified baseline and ownership

Inspected `edge` at `5939ae4924e901df27107ceeb3503d3e342164c0`, EONW **v44**.
The owning epic is `eon-remote-workspaces-52i4.6`; contract approval is `.11`
under `eon-remote-workspaces-52i4`, followed by `.6.1` checkpoint, `.6.2`
restore/reconcile and `.6.3` degraded/CLI/proof. Their existing splits and
assignment rules remain authoritative. Slice 2a is landed; the `eon-hyil8`
dependency has been removed. This slice needs neither a global session manager
nor migration between windows.

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

Use `recovery.checkpoint` inside the **existing per-window runtime directory**,
beside its management records. This is same-boot/login state, not reboot
persistence. Use the existing secure runtime-directory ownership rules,
regular no-follow files owned by the user with mode 0600, and bounded reads.
Unknown versions, duplicate IDs/references, unsafe type/owner/mode, wrong
namespace/generation or malformed structure never become partial topology.

The following is the proposed logical schema, not a new source of wire or
process authority. Format version 1 has these required fields:

| Field | Meaning |
| --- | --- |
| `format_version`, `revision` | Version 1; monotonically increasing checked u64 commit revision |
| `namespace`, `generation` | Exact C15 recovery target, validated independently |
| `created_build`, `active_build` | C10 lifetime provenance; not aliases for generation, identity or recovery permission |
| `sessions` | Ordered session entries: original local `sN`, recorded confirmed persistent ID or null, ordered tabs |
| `tabs` | Original `tN`, launch-directory policy and directory bytes, ordered columns, persistent popup slots, last valid selection |
| `columns` | Ordered pane references and expansion state, with current layout invariants |
| `panes` | Original `pN`, pinned-name state, exact terminal/run reference and start-intent or confirmed-run evidence |
| `popups` | Original `uN`, catalog key and saved display label/directory attributes, exact terminal/run reference and start evidence |
| `selection` | Last committed shown session, active tab, selected pane/popup references, all checked against this topology |
| `terminal_id_high_water` | Maximum terminal identity allocated, including durable intents with no remaining record |

Arrays belong to their parent; references cannot cross sessions or duplicate
a terminal/run in two slots. Paths use explicit base64-encoded raw Unix bytes,
not lossy display strings; decoded bounds are the existing path bounds.
Session names come from the registry, not an authoritative checkpoint copy.
Terminal provenance (`origin_build`, `record_generation`, protocols) comes
from validated records; unknown provenance stays null, never inferred from
checkpoint build fields. No terminal screen/history, input, command to replay,
attention/read acknowledgement, device-local scroll/selection or transient
picker request is stored. Orbit retains its live screen and history.

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
records. A damaged checkpoint cannot supply a trusted high-water; degraded
recovery uses C15's independently acquired terminal inventory. Exact run IDs
and fresh local handles, not an untrusted numeric claim, prevent retargeting.

### Commit and effect semantics

A durable commit writes a fresh temporary file, fsyncs it, atomically renames
it over the valid checkpoint, and syncs the parent directory **before** success
is replied. Runtime serializes revisions on bounded off-control-loop I/O;
stale completion cannot overwrite newer state or republish a closed target.
Terminal I/O, Inspect, navigation and canonical Stop remain serviceable.

A Start first reserves its exact slot, terminal ID and opaque run ID and
commits that intent. Only then may Runtime create the empty management record
and spawn. Orbit publishes Ready, not Runtime. Confirming the same run updates
the checkpoint; recovery never repeats Start. Failure before durable intent
has no spawn effect. Failure after a process/registry effect reports the
actual effect with `durability: "failed" | "uncertain"`, exit 1 and
`code: "checkpoint-failed"`; it never claims rollback or invites blind retry.
A lost reply remains unknown outcome under C10, exit 4.

Structural commits cover tab/pane/popup create and close, pane rename/pin,
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
   names, persistent popups and valid selection. Registry lookup failure does
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

Selection restoration takes no desktop focus. An invalid selection chooses
the first surviving structural target in stored session/tab/column order;
input remains disabled on stopped/inert targets and is never redirected to
another run merely because the old target vanished.

### Damaged, missing or over-capacity checkpoint

After safe full-set C15 acquisition, checkpoint damage, unknown format,
missing required state or topology-plus-extra capacity overflow uses the same
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
and fresh commit reopen creation, not the old topology; the recovery diagnostic
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
  degraded reason distinguishes absent, invalid, unsupported, over-capacity or
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
may extend that versioned envelope with its own approved transition and
mutation-dedup section. This draft excludes replaying input, commands,
transient UI or historical events, **not** the separately planned preservation
of mutation-dedup results across Update. Unknown required sections refuse
interpretation rather than silently dropping transition state. 2b neither
writes an Update section nor implements its consumer. Crash recovery retains
current same-build admission; planned compatible Update must prove its own
transition and cannot bypass C15 by presenting this file.

C10's present exclusion of durable layout restoration and recovery of pane
pins gains only this proposed C15 exception. Pane tags are not added to this
slice's saved fields; their current non-restoration rule remains unchanged.

Amend C15/C33 for checkpoint/reconciliation; C8 for explicit consistency with
last-live exit, not a new empty-window exception; C10/C39 for shared projection
and outcomes; C11/C17/C18 for durable structural action and no picker replay;
C38 for exact registration reconciliation. **C42 is already Planned on edge**:
a validated checkpoint can restore the original session/in-use tie, but a
flat/degraded projection cannot prove that an old registration is unused.
Do not allow Forget from that uncertainty. Forget itself stays outside 2b.
No new contract ID is allocated. Existing proofs keep their recorded scope.

## Acceptance matrix

All rows are **NOT RUN**. Documentation review does not prove runtime behavior.
Implementation records exact revision/component/protocol versions, isolated
installed platform, commands, artifacts and observed identities/counters.

| ID | Required observation |
| --- | --- |
| R1 | Three registered sessions, tabs/columns, pin, popup and selection survive exact supervisor SIGKILL and separately exit without Stop; identical run/PID/start identities and advancing counters |
| R2 | Crashes before intent commit, before record creation, after empty record creation, after Ready and after confirmation; absence evidence vs whole-set unsafe-record refusal; no duplicate Start |
| R3 | Corrupt/unknown/missing/wrong-owner/path checkpoint: all safely acquired runs usable; preserved original bytes/path, fresh registration and fresh writable checkpoint; failed rename/fsync stays write-refused |
| R4 | Valid topology plus extras exceeds capacity: same degraded path; too many live runs refuses whole set; no truncation, loss or extra Stop |
| R5 | Restored ended/start-unconfirmed placeholders, inert tabs and catalog-missing live popups; no picker/command replay; close placeholder sends no Stop |
| R6 | Sibling exit preserves stopped slots; final live exit ends window; no-live acquisition opens no recovered window and preserves diagnostic |
| R7 | Inject failures at file write/fsync/rename/parent-sync and registry-then-checkpoint; report real effects/durability; stale worker/picker completion cannot republish after Stop |
| R8 | Every enumerated structural action commits before success; selection coalescing bounded under continuous changes; control-loop I/O, Inspect and Stop stay serviceable |
| R9 | Full/partial canonical Stop and failed checkpoint clearing: exact stopped targets, checkpoint_cleared, retained owner only for unfinished process cleanup |
| R10 | Duplicate/cross-session refs, wrong generation, exhausted revision/IDs/size, replacement local-ID ranges and terminal intent high-water; no old ID retargeting |
| R11 | Registry pending/failure/retry preserves exact IDs and old registrations; extras follow originals; degraded state cannot prove old registration unused for C42 |
| R12 | Inspect/list, initial/gap/live C39 projection and exit codes agree; null stopped endpoints, distinct diagnostics, fresh STREAM and no event/input replay |
| R13 | Shared topology format review with Update owners; independent Update transition/dedup proofs remain pending and cannot be claimed by 2b proof |

## Reference and verification notes

PR #2 is closed/superseded; its source at
`610b053c184f66144b75bcc9b73dc44f44a2561e` is reference for topology/checkpoint
ideas only, not imported implementation or transferable proof. Current edge
C10/C11/C15/C33/C38/C39/C42 and the owning beads take precedence over that
prototype and the previous draft. The [remote plan](REMOTE-WORKSPACES.md)
continues to sequence 2b before SSH attach and later takeover. No runtime
refresh, implementation test or live-session disruption is part of this
planning revision.
