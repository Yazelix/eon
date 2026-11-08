# Recoverable sessions: identity and later recovery

[EON-C38](CONTRACTS.md#eon-c38--persistent-workspace-identity-and-host-registry)
was approved as Planned on 2026-11-03. Slice 2a
(`eon-remote-workspaces-52i4.5`) implements persistent IDs and optional names
for this user's host registry. Each window still owns its session members;
its `sN` handles map to durable IDs. The CLI reads registrations independently
of a running window and targets a member by its full persistent ID.

This fork's integration acceptance and independent reviews remain pending.
The registry stores neither membership nor layout. Restart recovery still
merges surviving terminals into a session that remains usable by its local
handle with a null persistent ID until registration confirms a new ID.
Registration errors are visible without stopping recovered work; restoring original
groups belongs to slice 2b. The remote flows below remain a plan and future
acceptance matrix, not implemented SSH or recovery guarantees.

## Office to home, with one active PC

`homePC` stays connected to the server at the office. On arriving home, the
user can continue the same session from `linglong` without disconnecting
`homePC`. The [accepted flow](https://github.com/Yazelix/eon/pull/3#issuecomment-6596829642) follows
[Lucca's simpler model](https://github.com/Yazelix/eon/pull/3#issuecomment-6595927531):

- Both PCs keep their connection, but a session has only one active client.
  The active PC receives live terminal output, sends program input and supplies
  terminal sizes. Orbit remains the sole terminal-state/PTY authority.
- The other PC keeps its last view frozen and dimmed, with a notice naming the
  active PC. It is not a live output monitor or a second input source.
- A key or click in that inactive session requests takeover. That first
  interaction only takes over; it is not sent to the program. After successful
  takeover, redraw from current authoritative state at the new PC's sizes
  before accepting subsequent program input. Returning to the office works
  the same way, without an explicit detach/reattach step by the user.
- Each PTY still has one size. Only the active client's sizes apply; a parked
  client's resize, background output, hover, local scroll, reconnect or
  automatic foreground restoration does not take control or resize a PTY.
  Takeover can make a TUI reflow. The triggering key/click, including its
  associated release/repeat or pointer phases, must not leak into the program.
- Local focus, scroll and selection belong to the device. They may be restored
  if their exact targets remain valid, but the frozen image is not current
  state. Takeover must refresh first: a deleted pane or replaced target cannot
  receive stale input. A paste or compound gesture used to activate cannot
  leave residual input to be replayed after takeover.

This replaces this PR's earlier simultaneous-live-client plan. The accepted
current direction is one active client with connected, frozen inactive views;
it is not described as a promise to add multiple simultaneous controllers.
Takeover needs its own exact contract amendments and proof after SSH attach.
Today's ORB-C3 still admits only one interactive peer and one limited metadata
observer. That observer is not a frozen-window/takeover API or a live display
subscription. Naming another process with `AttachAllow` does not itself end
an existing attachment, so switching safely is not already implemented.

## One host per window

Yu also accepted [one machine per window](https://github.com/Yazelix/eon/pull/3#issuecomment-6596085306).
Multiple machines use multiple windows; each session belongs to one host.
A failed connection therefore affects its host's window, not part of a mixed
window. There is no cross-host retargeting of an existing window in this plan.

After SSH attach exists, the Sessions picker may become one place to find
sessions on other machines. Choosing one would open or focus that machine's
window. That later discovery slice does not introduce mixed-host tabs, a
second SSH selector, or a new global attention ordering. Sidebar/attention
navigation remains scoped to the window's host unless separately amended.

## Lifecycle and failure rules retained for their owning slices

- Closing a remote Eon window or explicitly disconnecting releases only that
  client's relevant connections. It stops no remote task and does not close the
  other PC's connection; Quit (Alt+Shift+Q, `eon quit`) still stops the remote
  terminals. A local window's close quits it instead (eon-7cf0y.16); that rule
  does not apply to remote windows. A parked window can still be closed locally without taking
  over. Takeover transfers interactive presentation/input, not Orbit's
  supervisor lifecycle lease or process ownership.
- Closing a pane stops its shared terminal; closing a tab stops its terminals.
  Session Stop addresses one explicit host/session ID, not every session
  in the window or on a previously visited host. Other clients see the same
  lifecycle result when current state is presented. A parked terminal image
  stays visibly stale; authoritative lifecycle/connection notices may change
  without treating that frozen image as live. On takeover, ended targets are
  reconciled before input. The final Stop leaves an empty view and picker.
- A failed/partial Stop retains the remaining work. A request whose reply was
  lost has an unknown outcome until inspected; losing a connection does not
  cancel a server-accepted action. Offline input and Stop are not queued or
  replayed. Forget requires authoritative confirmation that work has stopped;
  an unreachable host or cached-stopped observation is insufficient.
- Supervisor exit/crash recovery restores the original identities, membership
  and layout around verified surviving Orbit runs. It does not replay commands
  or claim to resurrect processes after Orbit death or host reboot. Missing
  runs become explicit stopped slots; invalid/unknown snapshots preserve live
  work and diagnostic state instead of fabricating success.
- Build compatibility means equality of EONW, Orbit management, ORBS and ORBF
  versions. A compatible build can reconnect; a mismatch fails clearly without
  stopping, replacing or upgrading live work. Keep remote UID, PID/start-time,
  socket inode and lease checks: forwarded socket credentials prove the SSH
  hop, not the identity of the remote terminal owner.
- Portable hosting must stand alone and have installed proof without a systemd
  user service. Managed hosting is explicit opt-in, never inferred from systemd
  being installed, and never silently falls back. Both use lock-and-recheck
  bootstrap and the same recovery owner. Connecting installs nothing, enables
  no linger and upgrades no live work. Changing hosting policy must not kill
  or silently replace live work. Last-logout survival depends on the host's
  process/runtime-directory policy and must be diagnosed honestly.
- Nix/Home Manager is part of the initial remote feature, including a headless
  server package without Venus or graphical resources. Managed services have
  no graphical-session binding, and imperative setup never overwrites Home
  Manager-owned units. Independent Orbit lifetimes survive supervisor service
  cleanup; GC roots/profiles keep needed
  store resources alive through GC, switch, rollback and compatible upgrades.
  Runtime does not query or persist Nix store paths; retain one component graph.
- Every new user action has an ID-targeted `eon` CLI path in its owning slice,
  amending EON-C10 as needed. Cached remote state is labelled in both CLI and
  presentation. A failed remote lookup never becomes local session creation.
  Background connection/output is not proof that attention was seen; this plan
  adds no cross-device OS-notification or global read-state scheme.

## EON-C38 — Persistent workspace identity and host registry

**Approved by Lucca on 2026-11-03 and indexed in
[`docs/CONTRACTS.md`](CONTRACTS.md#eon-c38--persistent-workspace-identity-and-host-registry)
as Planned.** That entry is canonical; the text below is the reviewed draft.
It is not an implemented or proved guarantee.

- **Status:** Planned (approved 2026-11-03; see `docs/CONTRACTS.md`).
- **Consumer:** Local and remote Eon workspace clients and the host's
  ID-targeted workspace operations.
- **Trigger:** Create or rename a workspace, list/read its registration, resolve
  a target, detach/reattach, or restart the workspace supervisor.
- **Result:**
  - The host owns a durable, opaque workspace ID and a versioned registry.
    An ID identifies the same registered workspace across client and supervisor
    lifetimes. A window's `wN` is a local handle, not durable identity. A remote
    target includes its host and workspace ID; one window still uses one host.
  - Names and directories are attributes. Renaming keeps the ID; names need
    not be unique. Separate workspaces may use the same directory, and one
    workspace may contain multiple directories. A stale/deleted ID is never
    rebound to a different workspace merely because its name/path matches.
  - Registry inspection can find registered identities independently of the
    current window's members. Registration is metadata, not proof that any
    terminal is running or that a recorded process may be signalled.
  - Concurrent registry mutations are serialized. A successful durable mutation
    is acknowledged only after atomic persistence; failure does not publish a
    successful new identity or partly committed registration. IDs are not reused.
    Version changes preserve the old durable file as the repository's durable
    format policy requires; no unknown version is silently overwritten.
- **Important failures:** Unknown/stale/ambiguous targets, corrupt or unknown
  registry versions and persistence failures give explicit diagnostics rather
  than guessed targets or a replacement with the same name. Malformed state
  remains available for diagnosis. Registry failure authorizes no terminal
  stop, destructive cleanup, duplicate process launch or fallback local creation.
- **Owner:** Eon Runtime owns persistent IDs, registry state and target
  resolution. Eon projects that authority through its existing command/picker
  boundaries. Orbit remains the authority for terminal/run state and lifecycle.
- **Boundary:** Identity and registry only. Slice 2a does not promise original
  membership/layout recovery, SSH transport, takeover, terminal stop/Forget
  behavior, reboot process restoration or moving live work across hosts. Any
  CLI/schema changes introduced with 2a amend EON-C10 in that slice; this text
  allocates no separate CLI contract. Existing window-local handles may remain
  adapters, with an explicit mapping to persistent IDs.
- **Proof:** Not run. Slice 2a requires I1–I3 in the acceptance matrix below.
  Original groups/layout and verified run recovery belong to slice 2b and
  amendments to EON-C15/C33, not this proof.

## Order and contract routing

The next implementation sequence is **2a → 2b → 3 (SSH attach) → takeover**.
Only the next slice's contract text is brought for approval now. The remaining
rows are a plan, not approved future contract text or implementation claims.

| Phase | Scope and where its text belongs |
| --- | --- |
| 2a | Persistent identity and host registry: new EON-C38, after exact-text approval; CLI changes amend EON-C10 |
| 2b | Local original-group/layout recovery: amendments to EON-C15 and EON-C33 |
| 3: SSH attach | OpenSSH, headless host and portable hosting: use reserved EON-C34; incorporate existing supervision/attach beads |
| Takeover, after 3 | Connected frozen clients, single active client and safe switching: exact Eon/Orbit/Venus amendments approved in that slice |
| Managed hosting | Explicit service ownership and Nix/Home Manager support/proof; part of the initial remote feature, not silently deferred away |
| Discovery | Existing fzf picker/host-label work, host/offline states and ID-targeted CLI; other hosts open/focus their own windows |
| Incremental | Canonical incremental frames, bounded backpressure and authoritative resynchronization |
| History/performance | Bounded history, reading-anchor invalidation and measured end-to-end network proof |

Exact dependencies of the follow-ups stay with their owning slices. Existing
`eon-remote-ssh-x7dr.2`, `eon-remote-workspaces-52i4.2` and `.3` are reused for
supervision/attach and discovery/host-label work, not duplicated. Their old
attach-first/exact-build wording is not the current plan. PR #2 remains a
prototype/reference; its code or proof is not imported wholesale.

C34 is reserved for SSH attachment and will be used for that feature. C38 is
only the new identity/registry contract submitted for review here. Recovery,
CLI, close and final-window behavior amend EON-C15, C33, C10 and C8 in their
owning slices; attach/window/attention/platform boundaries are amended there
as necessary. No other new contract IDs are allocated by this plan. Orbit
retains input, admission, viewport/selection and frame/history ownership;
Venus retains presentation. Old proof remains valid only for its recorded
surface, not for these future changes.

This revision was checked against edge
`a92831765c0af35f112b3f17adcc01ecdddde2be`; EONW/management/ORBS/ORBF versions
are 29/5/14/3. Implementation must use the then-current canonical schemas.
The original PR base remains `3ee3422`; repairs append commits without rewriting
published history. The existing slice-1 bead is `eon-remote-workspaces-52i4.4`.

## Acceptance matrix

**All rows are future runtime proof, NOT RUN.** Every accepted proof records
exact source/component/wire versions, artifacts, platform, hosting mode and
commands/observations/results. Use independently identified sibling sessions
and hosts to prove isolation. Process identities and advancing counters prove
continuity; screenshots or reconstructed layout alone do not. Portable and
managed each need installed evidence. No prototype or unrelated platform proof
transfers; Orbit death and reboot remain outside original-process continuity.
The final history/network proof is needed before issue #1 is complete.

| ID | Phase | Scenario / cheapest falsifier | Required outcome |
| --- | --- | --- | --- |
| I1 | 2a | Two sessions with one directory/duplicate names; rename; reopen registry and restart supervisor | Distinct persistent IDs; rename preserves ID; registrations survive; no claim of layout/process recovery |
| I2 | 2a | Concurrent registry mutations; interrupted/failed durable write; creation commits but its reply is lost at a request/reply boundary | Serialized atomic state or explicit failure; no reused ID or partly successful registration. A missing reply does not imply rollback. If the committed result cannot be reliably identified, report uncertainty without guessing from possibly duplicate names; do not automatically reissue an uncertain creation as a new request |
| I3 | 2a | Corrupt/unknown registry version, stale/ambiguous ID/name | No overwrite, guessed/rebound target, destructive cleanup or duplicate launch |
| I4 | Discovery/lifecycle | Forget live, confirmed-stopped, cached-stopped and unreachable sessions | Only authoritative confirmed-stopped registration can be forgotten |
| R1 | 2b | Normal supervisor exit and replacement with multiple layouts/popups | Original IDs/groups/layout and verified live Orbit/PTY identities; counters advance |
| R2 | 2b | Supervisor crash and concurrent replacements | One supervisor/lifecycle ownership winner; no duplicate terminals or split lease |
| R3 | 2b | Corrupt/unknown snapshot; missing/extra run | Honest blocked/partial recovery; missing slots stopped; no unsafe adoption, restart or Stop |
| R4 | 2b/3 | PID reuse, wrong UID/start-time, replaced inode or stale lease | Refusal without signal, deletion or attachment to substituted owner |
| L1 | 2b/3 | Detach (Alt+Shift+D) a local window, or close or detach a remote window; reconnect repeatedly | Work survives; same persistent target and fresh authoritative display |
| L2 | 3/Discovery | SSH loss; input/Stop while offline; reconnect | Disabled/rejected offline actions; no replay; process counter continues |
| L3 | Lifecycle/Discovery | Stop session A while another host is open in another window | Only A stops; unrelated session/host tasks remain; final Stop retains empty window/picker |
| L4 | Lifecycle | Partial Stop or lost acknowledgement | Remaining work retained; explicit partial/unknown outcome; query before further action on that target; unrelated work remains usable; no automatic replay |
| L5 | 3, before takeover | Second interactive attachment without takeover support | Explicit busy result preserves the existing attachment and work; no silent takeover |
| M1 | Takeover | A active, B connected/frozen; B presses a key or clicks, then A does likewise | Both stay connected; exactly one active; parked view dimmed/names active PC; triggering interaction consumed; new active view freshly redrawn at its size |
| M2 | Takeover | Parked resize, hover/local scrolling, background output, reconnect or automatic restore | No takeover or PTY resize; terminal image remains frozen and labelled, not presented as live |
| M3 | Takeover | Active PC resizes then disconnects | Only valid active resize applies; last PTY size remains until explicit activation; truthful disconnected/no-active status; no task Stop or unattended stale-input replay |
| M4 | Takeover | Key press/release/repeat, paste and pointer sequence at activation, with resize | Entire activation interaction/residual phases consumed; no program input until activation/redraw coherent; stale mouse coordinates rejected |
| M5 | Takeover | Competing takeover requests; old active sends late input; target deleted while parked | One authoritative active client, old input fenced; stale targets rejected before input; no ID rebinding or phantom terminal recreation |
| M6 | Takeover/lifecycle | Pane/tab close or explicit session Stop succeeds, partially fails or loses reply | Exact shared target only; authoritative stopped/partial/unknown result; parked image not claimed current; reconcile before takeover input |
| M7 | Discovery/Takeover | Parked device/control status and attention; other host disconnects in another window | No inferred read acknowledgement/notification-owner transfer; lifecycle notices truthful; other window/host remains isolated |
| M8 | Takeover | Parked/active window closed or disconnected while the other connection remains; repeated returns | Local disconnect stops no remote task/other connection; explicit activation resumes usable current state; bounded clients/queues and no master leaks |
| S1 | 3 | Configured hosts and ProxyJump, using separate host windows; remote Project/Git/Agent/panes | Venus local; directories/work remote; OpenSSH config respected; no mixed-host window |
| S2 | 3 | Wrong host key, missing batch authentication or forwarding failure | Clear refusal; no host-key bypass, unintended remote mutation or unrelated-master cleanup |
| S3 | 3 | Different build/equal tuple; mismatch each of four versions individually | Equal tuple accepted subject to identity; every mismatch refused without stopping live work |
| S4 | 3 | Concurrent cold bootstrap and readiness failure | Lock-recheck yields one supervisor; no fallback duplicate or false readiness |
| S5 | 3 | At least three reconnect rounds and partial-startup failures | No accumulating masters/forwards or unintended cleanup of another window |
| H1 | 3 | Installed portable host without systemd user service | Attach/detach and supervisor exit/crash recovery independently pass |
| H2 | 3/Managed | Last logout under permissive and kill/remove-runtime policies | Survival where supported; otherwise diagnosed requirement, no implicit policy change |
| H3 | Managed | Managed service fails; systemd present but portable explicitly selected | No silent fallback; explicit selected startup owner respected; changing modes never kills/replaces live work |
| H4 | Managed | Restart supervisor service while Orbit/task counter runs | Independent Orbit/PTY survives with original identity; layout recoverable |
| H5 | Managed | Nix GC, HM switch/rollback and compatible upgrade with live tasks | Headless package excludes Venus/graphics; no graphical-session binding or overwrite of HM units; resources rooted and tasks preserved; runtime no Nix/store query or durable store identity |
| D1 | Discovery | Fzf and ID-targeted CLI listing/inspection/actions | Same host/session targets, freshness and outcomes; each new action has CLI parity |
| D2 | Discovery | Failed/offline host lookup with unmatched query; known cached row selected | No accidental local/replacement creation; cached explicit; exact ID resolved; other host opens/focuses its window |
| D3 | Discovery | Sidebar/attention and navigation through host/offline/stopped states | Truthful host-scoped status; no cached attention shown as live or mixed-host attention queue inferred |
| D4 | Discovery | Background agent create/rename/Stop/Forget and explicit window/session selection | Unrelated view/focus preserved; explicit movement only; typed failures affect no other target |
| P1 | Incremental | Representative output/graphics/resources against full-frame baseline on active client | Same authoritative visible state; missing/out-of-order deltas resynchronize; no live frame fanout to parked PCs promised |
| P2 | Incremental/Takeover | Slow/stalled active receiver and subsequent takeover/reconnect | Bounded queues/memory; coherent fresh resync; parked client cannot block owner or leak stale input |
| P3 | History/performance | Read anchors under output/pruning/reflow and RTT/bandwidth constraints | Stable reading or explicit invalidation, bounded paging and measured bytes/CPU/latency/queue comparison |
