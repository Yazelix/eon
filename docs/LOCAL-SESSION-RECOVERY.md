# Slice 2b: recover original local sessions and layout

**Draft for review, 2026-11-10.** This proposes the slice 2b amendments for
`eon-remote-workspaces-52i4.6`; it does not amend the accepted contracts in
[CONTRACTS.md](CONTRACTS.md). Yu approved three product directions: visible
safe degraded recovery, restoration of original structure and safe selected
state, and local same-boot recovery without cross-version takeover. The
behavior details and exact schema remain unapproved pending review with Lucca.
No slice 2b behavior is implemented or runtime-proved by this document. Every
acceptance row below is **NOT RUN**. Do not mark the proposed clauses Approved
or Proven before their required review and evidence.

This document is linked from the [remote plan](REMOTE-WORKSPACES.md). It is a
starting point for review and revision, not authorization to implement, merge,
or change the later SSH/takeover scope.

## Verified baseline

- Inspected upstream `edge` at `12ee64fc6d7d9997ecb5535b7f528eb2519912e3`.
- Owning bead: `eon-remote-workspaces-52i4.6`, open. Slice 2a (`.5`) is closed and landed as `a20504d` plus `c5f40f0`; EONW is v38. The current product terms are **session**, `sN`, and `terminal-N`, not the prototype's workspace/`wN`/`session-N`.
- Slice 2a's registry contains identity and optional name, not membership or layout. Registry failures do not prevent already-live terminal recovery or hold the supervisor control loop. This is an important preserved behavior, not an incidental implementation detail.
- C15 still recovers full Eon into synthetic `t1`; C33 still merges into one `s1`. C15 is only partially proved on the current component set. Its older pre-Bend proof does not prove slice 2b.
- The `eon-hyil8` dependency was removed on 2026-11-09. This slice can remain within today's window-owned session collection. It does not need a global session manager or session migration between windows.
- PR #2 is closed as superseded, but its source at `610b053c184f66144b75bcc9b73dc44f44a2561e` remains useful reference material. Its proof is specific to that old branch.

## Accepted directions and remaining contract details

1. **Damaged checkpoint: usable recovery. Yu approved this direction.** If existing C15 validation can safely recover the runs, expose them in a clearly labelled synthetic recovery session and show the checkpoint error. Do not guess original membership or bind a saved ID based on a name/path. Preserve the damaged file and original registrations. A strict refusal would preserve processes but leave them unavailable through the normal UI until the metadata is repaired. This is the biggest behavior choice because the prototype refused before recovery, while the landed 2a explicitly protects terminal availability through registry failure.
2. **Restore the original structure and safe selected state. Yu approved this direction.** Session membership and order, persistent session identity/name, tab order and launch-directory policy, column/pane order and expansion, pinned pane names, persistent popup membership, and the last valid shown/active/selected targets. Do not promise device-local scroll/selection, zoom, old attention/read acknowledgements, or restored request history. Terminal screen/history remain in the surviving Orbit. A running Files/directory picker or other transient action is not replayed.
3. **Keep 2b local and same-boot, without cross-version takeover. Yu approved this direction.** Retain C15's same-boot/login, exact recovery namespace/generation and existing admission checks in this slice. Do not introduce cross-build recovery, SSH, logout survival, or takeover here. The adopted equal-wire-version compatibility policy remains the direction for its later owning slice; a new build must not gain recovery authority just because a file exists.
4. **When is a layout mutation durably successful?** Recommended: explicit structural changes report success only after the corresponding recoverable checkpoint commits, with bounded worker-owned I/O. A start's reserved slot/run identity must be durable before the run can publish Ready. If a process effect already happened, a subsequent persistence failure reports that effect and the failed/uncertain durability; it does not claim rollback. Selection changes may be coalesced, but the recovered selection is only the last committed one. This avoids synchronous disk writes on every keystroke while giving “successful structural change survives a crash” a precise meaning.

The first three no longer need to be asked again. The fourth and the precise stopped-slot, capacity and degraded-operation rules below are concrete recommendations for Lucca’s exact-text review. Filesystem and process operations are not one transaction.

## Proposed C15 amendment

Retain C15's existing authority, Ready, lease, bounded enumeration, cleanup and failure rules except for the explicit topology/recovery changes below. Do not replace its historical proof record or label the new clauses Proven.

### Consumer and trigger

- **Consumer:** One local full-Eon user, or an agent acting for that user, resuming the same window's work after its supervisor exits or crashes, on the same boot and login. The eonbase single-terminal recovery behavior is unchanged.
- **Trigger:** A replacement supervisor targets the same recovery namespace/generation after the previous lifecycle owner is gone. A normal explicit Stop or window Quit is not a request to restart stopped terminals.

### Result

- **One recovery owner.** Runtime owns one versioned recovery checkpoint for the window's session collection. It refers to C38's persistent session registrations and records the recoverable structural state and exact terminal/run membership. C38 remains the owner of registration and names; Orbit remains the only owner of terminal state, process identity and lifecycle authority. The checkpoint does not enumerate every registration on the host as a window member.
- **Durable structure.** The checkpoint records the session membership/order and confirmed persistent IDs; tab order and each tab's launch-directory policy; pane/column order and expansion; pinned pane names; persistent popup membership and launch-directory attributes; and valid last-committed shown, active and selected targets. It records exact terminal IDs and opaque run IDs sufficient to reconcile references, and the identity-allocation state needed to avoid reusing unresolved slots. It does not persist terminal memory, screen/history copies, commands to execute, queued input, request replay state, device-local scroll/selection, cached attention or foreground-process claims.
- **Publication.** Explicit structural mutations are acknowledged as durable only after an atomic, versioned checkpoint commit. Concurrent checkpoint work is serialized by Runtime and handled off the supervisor control loop; terminal I/O, inspection, attention and canonical Stop remain serviceable. Stale worker completion cannot overwrite a newer committed state. An interrupted write leaves either the previous complete checkpoint or the new complete checkpoint, never a partly decoded topology. Unknown or malformed files are retained for diagnosis and are not silently overwritten.
- **Creation boundary.** The intended exact slot/run identity is committed before a new terminal can publish Ready. Recovery never repeats a Start operation. A crash before spawn therefore leaves a stopped/unresolved slot, not a new terminal; a crash after Ready is reconciled with the same run. An unresolved launch outcome is not evidence that no process exists and does not authorize reusing that identity or launching a substitute.
- **Live reconciliation.** The replacement retains the existing C15 acquisition rules and bounds. It reconstructs the saved groups only around runs independently validated through canonical Orbit management, including the required UID, PID/start identity, record/endpoint identity, generation and sole lease checks. A saved path, PID, `live` bit, name or registry entry is never authority to attach, signal or delete anything. A slot becomes live only when both its terminal ID and opaque run ID match the validated run.
- **Missing and extra runs.** A valid saved slot whose run is definitively absent is displayed as stopped in its original position. It is not silently removed or restarted. Unresolved/unsafe identity remains explicitly unavailable, not falsely declared stopped. A validated run not claimed by the saved topology remains running; when existing C15 recovery permits publishing it, it is exposed separately in the synthetic recovery session rather than silently assigned to a similarly named session or stopped as an orphan. The slice does not relax C15's refusal of unsafe or partially acquired run sets.
- **Checkpoint failure.** Bad, unknown, missing-required or unavailable checkpoint state does not hide or stop runs that the existing C15 path can safely recover. Those runs remain usable through a synthetic session with a visible recovery diagnostic. No original grouping is claimed. Its synthetic member has `persistent_id: null`; it is not silently registered as a replacement for an old session. Damaged checkpoints and registrations are preserved. Checkpoint absence alone is not proof that the runs are legacy. Only an independently established legacy namespace may use the existing flat recovery and registration behavior; when that distinction cannot be established, use unregistered degraded recovery rather than allocating a guessed replacement identity.
- **Registry-only failure.** A structurally valid checkpoint may retain its verified grouping even when registration lookup is unavailable. The affected members use local handles with `persistent_id: null` and a visible registration diagnostic until the exact recorded registrations can be confirmed. Names that cannot be read authoritatively are not guessed from paths or another registration. Retry resolves the recorded ID, never a newly allocated or same-name ID. A missing registration does not invalidate the independently verified terminal runs, authorize removal, or silently rebind their identities.
- **No implicit replay.** Checkpoint recovery starts no terminal, picker, popup command or agent to fill a missing slot. Persistent popups are restored only around their verified surviving runs. Removing or changing their configuration does not reclassify those live tasks as disposable helpers. Existing canonical cleanup of specifically identified transient helpers retains its existing authority; checkpoint failure alone authorizes no additional Stop. A saved pane-free pending or popup-only tab without a usable run becomes an inert stopped/unavailable structural tab, not a resumed picker. Merely dismissing an old picker or presenting that tab does not start a shell. The user may explicitly close it or create/open new work once durable state is writable. Legacy recovery without a slice-2b checkpoint keeps C18’s existing stale-initial-pair cleanup/fallback; that exception does not apply to restoring a saved slice-2b tab.
- **Empty or over-capacity projections.** A valid checkpoint whose runs are all definitively absent retains its saved sessions/tabs as stopped structural slots and starts no shell. A checkpoint recording completed final close/Stop does not reopen those members; a normal fresh launch remains a separate action. Stopped and unresolved placeholders count toward the same 256 terminal-slot bound as live slots. If the original structure plus separately exposed extra runs cannot fit the existing member/slot bounds, preserve the checkpoint and show all otherwise safely acquired runs in one explicitly degraded synthetic group instead of hiding extras, exceeding bounds or claiming original-layout success. Its diagnostic states that original grouping and placeholder presentation are unavailable in this fallback, while the checkpoint is retained. **Open for review:** define whether structural writes are blocked during this valid-but-over-capacity fallback and which explicit action may replace the retained checkpoint. No automatic reset or replacement is authorized by this draft.
- **Degraded operations.** While damaged/unavailable state prevents a durable checkpoint, the recovered terminals remain usable for input, reading, inspection and canonical Stop. New launches and structural writes that require that checkpoint refuse with the recovery diagnostic; they do not silently replace the damaged file. Restoration of valid writable state is required before such writes can succeed. Actual process completion or Stop remains truthfully reported even if the checkpoint cannot be updated. Closing an exact live target may therefore stop it but return a partial/persistence failure; it must not report that the target is still running or that the complete structural action succeeded. The diagnostic supplies the recovery-state location and the reason it needs repair, without silently deleting, replacing or resetting it. An automatic repair/reset command is not added by this slice.
- **Shutdown effects.** Successful explicit close/Stop removes the affected target from recoverable membership without deleting its C38 registration. A later crash must not turn a completed close into an automatic launch. If Stop took effect but checkpoint persistence or its reply failed, report the observed partial/unknown outcome and reconcile the exact target before further lifecycle work. Never restore a process by replaying the failed action or use filesystem failure as permission to stop unrelated work.

### Important failures

Checkpoint size, bounds, duplicate identities, cross-session references, wrong namespace/generation, unknown format, wrong owner/mode/type, unsafe paths, unavailable directories, persistence failures, unresolved launches and stale worker completions produce an explicit diagnostic. Valid process continuity is never inferred from restored geometry. Existing PID/socket/lease refusals and cleanup restrictions remain in force. One metadata failure cannot stop a healthy sibling session or another window.

### Boundary and proof

Local, same boot/login, same existing recovery target only. No Orbit resurrection, reboot restoration, SSH, logout guarantee, global session manager, moving work between windows/hosts, multi-client takeover, remote discovery, Forget, or new installation/upgrade semantics. This slice does not change the local window-close/quit policy or introduce the later “final Stop keeps an empty remote window” policy.

Proof is pending under `eon-remote-workspaces-52i4.6`. Existing C15 proofs retain only their recorded scope. The acceptance rows below are NOT RUN for this draft.

## Proposed C33 amendment

Replace only the recovery portion of **Detach, attach and recovery**, preserving its current detach, local-close, window-Stop and Quit semantics:

> Detach and reattach to the same live supervisor retain its session collection and local-handle/persistent-ID mapping. After supervisor loss, C15 restores the original session membership and structural layout from a validated checkpoint around verified surviving Orbit runs. Confirmed persistent session IDs retain their identity; names are display attributes and do not select recovery targets. Local `sN` handles and other local presentation identities are exposed with the recovered mapping and must not be treated as host-wide durable identities. Recovery concerns this window's recorded members, not every entry in the host registry.
>
> Missing runs remain explicit stopped slots, and unresolved runs remain unavailable until reconciled. Recovery does not automatically prune these placeholders or recreate their terminals. Normal close/emptying after a live supervisor observes terminal completion keeps its existing semantics; registrations remain retained. A valid last-committed selection is restored without taking desktop focus. If its target is no longer available, the replacement selects a valid surviving structural target deterministically and reports the unavailable target rather than redirecting input to a replacement process.
>
> If original membership/layout cannot be trusted, C15's synthetic recovery session is visibly identified as degraded recovery and exposes no guessed persistent mapping. A registry/checkpoint error neither blocks otherwise-safe live-run recovery nor starts a replacement terminal. An attach to the recovered supervisor uses fresh authoritative snapshots and a new event stream; it does not replay old requests or attention acknowledgements.

**Boundary replacement:** Retain C33’s exclusions of showing/moving another window’s or generation’s session, mixed ownership, zero session members and reboot restoration. Replace its exclusion of “names or membership kept past the supervisor’s end” with “no names or membership reconstruction beyond the validated same-boot C15 checkpoint; retained C38 registrations alone do not establish membership.”

**Important boundary:** Keep the existing 32-member/window and 256-terminal-slot limits (including stopped/unresolved placeholders) unless a separately approved limit change is necessary. Stop/close remain scoped exactly as today's contracts specify. The broader planned remote lifecycle is not silently implemented through this recovery amendment.

## Necessary consistency amendments

These are part of making C15/C33 truthful, not separate features or new contract IDs:

- **C8:** Replace the unconditional synthetic-`t1` claim with “C15 restores saved topology when valid; flat synthetic recovery remains its explicit fallback.” Its pane-name sentence must stop saying pins always end with supervisor loss if pinned names are included in decision 2. Add the narrow stopped/unavailable-slot exception to normal ended-pane pruning. Preserve normal live-session close semantics.
- **C17:** Its synthetic-`t1`/replacement-launch-directory recovery rule applies only to flat fallback. Valid checkpoint recovery retains each saved tab’s explicit launch-directory policy, without inferring it from a pane’s current CWD. Replace the blanket no-persistence boundary with the C15 exception. An unavailable saved directory prevents a new spawn in it, not recovery of an already-live terminal.
- **C18:** Add the narrow checkpoint-recovery exception for a pane-free pending tab: stop/retire only the specifically validated transient helper under existing authority, retain inert structural state, and never replay its picker or implicitly launch a first pane. Keep the existing legacy stale-initial-pair fallback explicitly separate.
- **C11:** Replace `no persisted topology` in its Boundary with a reference to C15's limited same-boot checkpoint recovery. Do not widen generation admission or grant cross-build adoption here.
- **C38:** Keep registry records as identity/name metadata. Recovery checkpoints refer to them; avoid turning registry presence into live membership or process authority. Narrow the 2a blanket “all recovered members receive a fresh registration” description to legacy flat recovery, and describe confirmed original identities plus explicitly unregistered degraded recovery.
- **C10 and C39:** C10 also repeats the pane-name sentence saying pins end with the supervisor and are not restored; make the same narrow C15 exception as C8. If the implementation adds recovery state/diagnostics or stopped popup fields, approve their exact JSON, wire/event projection, failure code and exit mapping before code. Inspection, initial/gap snapshots and live updates must agree. A log-only warning is not sufficient for the user-visible/agent-visible degraded state promised above. Reuse existing fields where their meaning truly fits; do not overload `registration_error` to conceal a layout error.
- **C23 and VEN-C8:** Check for window ownership, no-live-run and pending/popup-only/stopped topology and rendering assumptions. Amend only an actual contradiction or new relied-on field. Do not add an invented new recovery contract ID.
- **Plan document:** The accompanying plan update records that 2a landed, links this draft, makes R1–R4's local scope precise, and separates local detach from future remote close/detach. SSH, takeover, managed hosting and performance rows remain future proof.

The proposed behavior is specified here, but it is not approved contract text or a complete wire/schema amendment. Before this draft can become an accepted contract amendment, it must include the exact recovery diagnostic/availability fields, their nullability, typed refusal codes, CLI exits and event-snapshot projection if these are new. Those exact shapes and the behavioral details need approval before implementation. Reuse current projections where possible; this draft does not authorize an unnamed protocol change or let those details be decided silently in code.

## Acceptance matrix

All rows below are **NOT RUN**. Proof must name the exact source/component versions, protocol versions, installed artifact, platform, command/observation and result. Use independently identified sibling sessions and another window. Process/run/PTY identities and advancing counters establish continuity; screenshots alone do not.

| ID | Scenario / cheapest falsifier | Required outcome |
| --- | --- | --- |
| R1 | Three named sessions, duplicate names/directories, multiple tabs/columns, pinned pane names and persistent Agent popup; supervisor exit without Stop, then replacement | Same confirmed persistent IDs, original membership/layout and committed selection; same Orbit/process/start/run/PTY identities; counters advance; no process reconstruction |
| R2 | Repeat R1 with a crash; race two replacement supervisors | One lifecycle/lease owner wins; no duplicate processes, split leases or changed sibling window; original work remains usable |
| R3a | Corrupt/truncated/oversized/unknown-format checkpoint, wrong owner/mode/type, wrong recovery namespace and duplicate references | Unsafe topology rejected and preserved; otherwise recoverable live runs remain usable in clearly degraded recovery; no guessed old ID, silent overwrite or destructive cleanup |
| R3b | Valid layout with one definitively missing run, one unresolved Ready intent and one extra validated run | Missing slot stopped; unresolved slot not falsely stopped/reused; extra run preserved and separately visible when safely recoverable; no startup replay |
| R4 | PID reuse, wrong UID/start identity, replacement record/socket inode, wrong run ID and Busy lease | Existing C15 refusals preserved; no signal/path/PID fallback and no attachment to substituted owner; unrelated work untouched |
| R5 | Crash before intent persistence, after intent commit/before spawn, after Ready/before final checkpoint, after commit/before reply | Zero duplicate starts; durable identity correlates with actual run; missing reply never implies rollback; unreconciled effects are explicit |
| R6 | Disk/rename/fsync failure or contention during create, move, rename and close; delayed/stale worker result | No false success or stale overwrite; actual process effects reported honestly; prior complete checkpoint preserved where possible; terminal input, inspect, attention and Stop remain serviceable |
| R7 | Registry root unresolved, unsafe, corrupt or locked; worker creation fails, with a valid or damaged layout | Valid verified grouping remains without unconfirmed persistent IDs; damaged layout uses explicit synthetic recovery; preserve landed 2a availability; no replacement terminal or silent ID reassignment |
| R8 | Persistent and transient popup recovery; configured popup removed; saved tab directory unavailable | Verified live tasks preserved; no picker/command replay; removed definition/path blocks only the new action that needs it, not existing live work; presentation shows unavailable state honestly |
| R9 | One missing slot, every run missing, pane-free pending tab, explicitly closed member, natural exit before crash, and completed Stop followed by crash | Saved stopped/inert structure stays visible without an automatic picker or shell; completed close/Stop does not silently resurrect work; retained registration is not treated as current window membership |
| R10 | Detach/reattach repeatedly; then supervisor loss; CLI inspect and event stream reconnection | Live detach preserves existing mapping; recovery exposes its confirmed map/diagnostics in CLI and UI; fresh event stream/snapshot, no stale mutation/input/attention replay |
| R11 | Proven legacy runs without a checkpoint, missing required new-format checkpoint, and ambiguous absence | Legacy-only behavior requires positive provenance; ambiguous/missing new-format state uses unregistered degraded recovery; all safely recoverable runs remain usable with no original-topology claim or guessed names |
| R12 | Maximum supported sessions/slots plus an extra validated run; panes/columns/popups, byte-path edge cases and ID allocator high-water marks | Explicit whole-projection degraded fallback when original layout plus rescue cannot fit; bounded decode, canonical references and capacity behavior; no reused unresolved identity; frame/schema representability verified |

For the implementation, the repository's lifecycle/wire verification route requires the affected Rust/Nix surfaces, the whole-workspace `nix develop -c eon-test` before review, and an isolated installed observation on the accepted final candidate. Any native macOS claim needs separate native evidence; this draft's installed baseline should be x86_64 Linux Wayland. Docs-only review requires source/contract/link/diff checks, not an invented runtime pass.

## What to take from PR #2

Adopt as a reference, not as code to apply unchanged:

- `workspace/layout.rs`: versioned structure; bounded private-file reads; structural validation; exact `(terminal ID, run ID)` reconciliation; atomic temp-file/sync/rename/parent-sync publication; identity allocator high-water marks; stopped placeholders; no persisted terminal memory or request replay.
- `supervisor/named.rs:709–761`: separate process effects from checkpoint publication; persist intended slot/run identities before a new run may publish Ready; retain slots until canonical Stop has acknowledged.
- `supervisor/named.rs:630–648`: an unresolved launch is not proof of absence and must not permit identity reuse or a duplicate start.
- Its old fault/continuity scenarios are useful test cases, but their previous pass does not prove today's Bend Orbit, EONW v38 or new worker-owned persistence.

Do not carry forward:

- A second named-workspace runtime/supervisor beside today's window/session owner.
- Its older EONW/Orbit management version numbers, `wN`/`session-N` terminology, endpoint naming assumptions, old C29 allocation or systemd-required hosting.
- Its “load layout before acquiring/recovering anything” path without deciding how to retain 2a's availability guarantee.
- Synchronous `save_layout` calls on the control loop.
- Its unclaimed-run stderr warning as the only user/agent visibility.
- Its global `stopped`/Forget behavior, different window Stop policy, remote UI, multiple presentation streams or compatibility relaxation. Those belong to later approved slices.

## Review and verification status

- Baseline: upstream `edge` at `12ee64fc6d7d9997ecb5535b7f528eb2519912e3`,
  rechecked on 2026-11-10. `eon-remote-workspaces-52i4.6` remains open;
  this proposal does not close or claim implementation of that bead.
- The draft was checked against the current contracts, plan, bead records and
  the referenced PR #2 mechanisms. A fresh read-only review is recorded in
  the PR, together with the final documentation checks and any limitations.
- No build, Rust/Nix suite, installed runtime observation or slice 2b acceptance
  test was run for this documentation-only proposal. Existing 2a and C15 proof
  retains only its recorded scope.
- Existing accepted contract text, generated agent instructions and changelogs
  are unchanged. The exact checkpoint format, diagnostic/availability fields,
  nullability, refusal codes, CLI exits and event-snapshot projection remain
  review gates; they are not delegated silently to implementation.

## Sources

- Current contracts: https://github.com/Yazelix/eon/blob/12ee64fc6d7d9997ecb5535b7f528eb2519912e3/docs/CONTRACTS.md
- Current plan: https://github.com/Yazelix/eon/blob/12ee64fc6d7d9997ecb5535b7f528eb2519912e3/docs/REMOTE-WORKSPACES.md
- Current beads: https://github.com/Yazelix/eon/blob/12ee64fc6d7d9997ecb5535b7f528eb2519912e3/.beads/issues.jsonl
- Slice 2a landing/failure behavior: https://github.com/Yazelix/eon/pull/6#issuecomment-6727991134
- Slice 2b request and dependency discussion: https://github.com/Yazelix/eon/pull/6#issuecomment-6728040997
- Yu's acceptance: https://github.com/Yazelix/eon/pull/6#issuecomment-6731634374
- Adopted split and compatibility direction: https://github.com/Yazelix/eon/pull/2#issuecomment-6469026815
- Prototype layout: https://github.com/bioinformatist/eon/blob/610b053c184f66144b75bcc9b73dc44f44a2561e/runtime/crates/eon-runtime/src/workspace/layout.rs
- Prototype supervisor: https://github.com/bioinformatist/eon/blob/610b053c184f66144b75bcc9b73dc44f44a2561e/runtime/crates/eon-runtime/src/supervisor/named.rs
