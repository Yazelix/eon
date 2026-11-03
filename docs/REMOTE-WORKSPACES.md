# Recoverable workspaces: contract proposal and acceptance plan

**Slice 1 is docs only.** This is proposed text for Lucca's review, not a claim
that remote or multi-client behavior is implemented or proved. It adds proposed
EON-C38–42 to [the contract index](CONTRACTS.md#eon-c38--host-owned-persistent-workspace-identity-and-registry)
and leaves existing approved contracts and their proof unchanged. No executable,
wire schema, component version, configuration or package changes in this slice.

## Goal and requested approval

Work keeps running on a headless server. A user can leave the office PC
connected, return home, and continue the **same workspace** on the home PC.
Both PCs stay connected, receive output and remain able to operate it; neither
requires the other to disconnect or surrender an exclusive control role. This
is one person's cross-device continuity, not a new multi-user collaboration
system. The server remains the sole workspace/terminal authority.

One window may also keep multiple explicitly selected hosts connected. Each
workspace belongs to exactly one host. Host connections, workspace attachment,
foreground focus and the terminal's process-lifecycle lease are distinct;
connecting a host does not attach all its workspaces or transfer its lifecycle
ownership. Per-device focus, scroll and selection are independent. Shared
workspace structure is authoritative on the host.

### Delta from the accepted plan

[Yu's October 28 plan](https://github.com/Yazelix/eon/pull/2#issuecomment-6468069685)
and [Lucca's reply](https://github.com/Yazelix/eon/pull/2#issuecomment-6469026815)
settled persistent IDs before SSH, split identity from original-layout recovery,
exact wire-version compatibility, portable/managed hosting, remote identity
checks, Nix resource lifetime and CLI parity.

That reply also explicitly required **one active client per workspace**, noting
that the prototype already accepted multiple presentation streams. The
multi-client direction above is a **requested change to that approved scope**,
not an interpretation that the earlier rule meant only one writer. Please
review this scope change along with its sizing/view and shared-close clauses
before implementation. The prototype's streams alone prove none of those
clauses. Existing ORB-C3 admits one interactive client plus one restricted
metadata observer; that observer is not an existing general attention or
multi-device display subscription API.

| Boundary | Previously approved/current baseline | Proposed behavior |
| --- | --- | --- |
| Same workspace on two PCs | One active client; conflicting presentation is busy | Both remain connected and interactive, with one authoritative server |
| Multiple hosts in one window | Unified model/discovery; simultaneous attachment scope not settled | Keep explicit host connections; changing foreground does not implicitly detach others |
| Device views and geometry | Existing one-client presentation | Independent device focus/scroll/selection; one size per PTY follows explicit active use |
| Close and Stop | Workspace-targeted Stop; window close detaches | Pane close stops its terminal, tab close its terminals, all devices see the result; window close remains local detach |

### Observable multi-client boundaries

- **Shared structure:** names, terminal identity and tab/pane membership belong
  to the host. Accepted creation/removal is visible to all attached clients.
  Each device keeps its current target if it survives. A removed target fails
  explicitly; no stale input/action redirects by name or goes to an automatic
  replacement. Server ordering is not collaborative shell-text editing.
- **PTY sizing:** each terminal has its own size source. Explicit pane
  activation immediately selects that device; actual key, paste and
  application-directed mouse input can also select it. Passive output,
  reconnect, automatic foreground restoration, local scroll/selection and
  hover do not. Only the current source's resize applies. The server applies
  the selected size before its accepted input and validates mouse coordinates
  against the applicable state. Other displays adapt locally; a shared TUI may
  still reflow. Disconnect retains the last size until explicit active use.
- **Lifecycle:** pane/tab close stops precisely its shared terminals; workspace
  Stop stops precisely the named workspace, across its observing devices.
  Window close/disconnect only releases that client's relevant attachments and
  connections. The last workspace Stop leaves an empty view/picker. Another
  host or unrelated workspace is untouched. No device-local pane-hide feature
  is implied. Existing confirmation/diagnostics must describe the shared scope.
- **Failure:** an accepted server action is not cancelled by client loss. Lost
  acknowledgement is unknown until queried. Offline input and Stop are never
  queued/replayed, nor is a stale action reinterpreted as a new target. Host
  failure is isolated; cached state is labelled. Remaining clients stay usable.
- **Bounds and attention:** multiple clients require bounded per-client queues,
  status work and resource cleanup, not unrestricted observers. The server
  stays authoritative for structured agent evidence. Background connection or
  output is not proof that the user saw an attention item. Exact cross-device
  read-state/OS-notification routing is not added by this proposal.

## Slices and approval gates

| Slice | Deliverable / boundary |
| --- | --- |
| 1 | Contract text, fresh IDs and this acceptance matrix; approval before code |
| 2a | Persistent IDs and host registry, shared by local and remote; no original-layout recovery claim yet |
| 2b | Local reconstruction of original groups/layout around surviving Orbit runs |
| 3 | Minimal OpenSSH attachment and independently proved portable hosting; incorporate existing attach/supervision beads |
| 4 | Explicit managed hosting and first-class Nix/Home Manager lifecycle/resource proof |
| 5 | Unified fzf discovery, host/offline states, attention and all new ID-targeted CLI paths; incorporate existing host-label bead |
| 6 | Incremental frames, bounded backpressure and authoritative resynchronization |
| 7 | Bounded history with stable/invalidation semantics and final network-performance proof |

Multi-client behavior changes the previous attachment assumption. Its owning
implementation slice and prerequisites must be agreed explicitly rather than
silently making all of it a slice-3 prerequisite. The persistent-ID/local
recovery foundation remains independently useful. Rows labelled expanded
attachment below are requirements for the proposed multi-client behavior,
not claims that an implementation slice is already approved or filed.

Existing `eon-remote-ssh-x7dr.2`, `eon-remote-workspaces-52i4.2` and `.3` remain
implementation material for slices 3/5, not competing tracks. Their earlier
attach-first/exact-build descriptions are superseded by the epic's October 28
accepted sequence, subject to this new scope review. Keep PR #2 as reference;
do not rebase or import the whole prototype.

The baseline for this proposal is edge
`3ee342294132c969f70d5cbece6a2b00b2378c95`. Its versions are EONW 28,
Orbit management 5, ORBS 14 and ORBF 3; each implementation slice consumes the
then-current authoritative schemas, not the prototype's numeric versions.
C29 is used/retired and cannot be reused. C34 was already reserved in Beads
for the previous SSH draft, even though not in the current index. C35–37 are
occupied. C38–42 are fresh proposal IDs at this baseline.

## Acceptance matrix

**All rows are future proof, NOT RUN.** Every accepted proof records exact
source/component/wire identities, installed artifacts, platform, hosting mode,
command or observation and result. Use independently identified siblings to
prove target isolation. Process identity and advancing counters establish
continuity; screenshots or a reconstructed layout alone do not. Existing
prototype or unrelated-platform evidence does not transfer. Portable and
managed hosting require separate installed proof. Orbit death/host reboot
are outside original-process continuity. Issue #1 is not complete before the
final bounded-history and network-performance proof.

| ID | Slice | Scenario / cheapest falsifier | Required outcome |
| --- | --- | --- | --- |
| I1 | 2a | Create two same-directory/duplicate-name workspaces; rename; reopen registry | Distinct stable IDs; exact target survives rename; directories remain attributes |
| I2 | 2a | Concurrent registry mutation; interrupted write | Atomic valid state or explicit failure; no ID reuse/duplicate successful creation |
| I3 | 2a | Corrupt/unknown registry or stale ID | No overwrite, guessed target, duplicate process or destructive cleanup |
| I4 | 2a/5 | Forget live, confirmed-stopped, cached-stopped and unreachable targets | Only authoritative confirmed-stopped target can be forgotten |
| R1 | 2b | Supervisor normal exit then replacement, with multiple workspace layouts/popups | Original IDs/groups/layout and verified live Orbit/PTY identities; counters advance |
| R2 | 2b | Supervisor crash; concurrent replacement contenders | One ownership winner; same live runs; no duplicate terminals or split lease |
| R3 | 2b | Corrupt/unknown snapshot; missing/extra run | Honest blocked/partial recovery; missing slots stopped; no unsafe adoption/restart/Stop |
| R4 | 2b/3 | PID reuse, wrong UID/start time, socket inode replacement, stale lease | Fail closed without signal, deletion or successful attachment to substitute |
| L1 | 2b/5 | Close Venus, detach and repeat reconnect | Work survives; same identity; fresh authoritative display |
| L2 | 3/5 | SSH loss; type/input/Stop while offline; reconnect | Input disabled; no queued input/Stop replay; process counter continues |
| L3 | 5 | Explicit Stop A after visiting B on another host | Only A stops; B remains running; last Stop retains window/picker |
| L4 | 5 | Stop partially fails or acknowledgement is lost | Remaining work retained; partial/unknown outcome; query reconciliation, no auto-replay |
| L5 | expanded attachment slice, requires approval | A/B both attach same workspace; alternate use without disconnecting either | Both stay interactive and receive authoritative output; no exclusive handoff; no duplicate Orbit/PTY |
| M1 | expanded attachment | A/B on same PTY explicitly activate/input in server-accepted order | One PTY size; latest accepted qualifying interaction wins; corresponding size applied before input |
| M2 | expanded attachment | A owns size; B scrolls/selects/hovers/resizes locally, reconnects or restores view | No size-source change; A/B local view state remains independent |
| M3 | expanded attachment | Current size source resizes, then disconnects | Only its valid current resize applies; last size retained on disconnect; tasks continue |
| M4 | expanded attachment | Different terminals have active devices A/B; application mouse input races resize | Each PTY source independent; stale coordinates fail explicitly; no misdirected input |
| M5 | expanded attachment | A/B mutate shared structure concurrently or act on a removed terminal | Server-authoritative resulting structure; stale target rejected; no ID rebinding or automatic process recreation |
| M6 | expanded attachment | Shared Stop/pane-close request succeeds, fails partially, or loses acknowledgement | Pane close stops only its shared terminal; tab close its terminals; workspace Stop its tasks; all clients see authoritative stopped/partial/unknown state |
| M7 | 5 / multi-client | Background host or second device receives attention/output | No implicit focus/read acknowledgement or notification-owner takeover; host failure isolated |
| M8 | expanded attachment | One device closes window/disconnects while another remains; repeated reconnects | Remaining client works; no remote Stop, stale input replay or leaked masters; bounded client resources |
| S1 | 3 | Two configured hosts/jump host; remote Project/Git/Agent/panes | Venus local, work and directory selection remote; OpenSSH config respected |
| S2 | 3 | Wrong host key / unavailable batch authentication / forwarding failure | Clear failure; no host-key bypass, remote mutation or unrelated master cleanup |
| S3 | 3 | Different build/equal tuple; independently mismatch each of four versions | Equal tuple accepted subject to identity; every mismatch rejected; no live task stopped |
| S4 | 3 | Concurrent cold bootstrap and readiness failure | Lock-recheck produces one server; failure cannot create hidden duplicate |
| S5 | 3 | Repeated reconnect and failed partial startup (at least three rounds) | No accumulating SSH masters, leaked forwards or unintended process cleanup |
| H1 | 3 | Installed portable host with no systemd user service | Attach/detach and supervisor exit/crash recovery pass independently |
| H2 | 3/4 | End last login under permissive and kill/remove-runtime policies | Survival where supported; otherwise honest diagnosed requirement; no policy mutation |
| H3 | 4 | Managed owner fails; systemd installed but portable explicitly selected | No fallback duplicate; explicit hosting choice respected |
| H4 | 4 | Restart managed supervisor while Orbit/task counter runs | Surviving independent Orbit/PTY identity, recoverable original layout |
| H5 | 4 | Nix GC, HM switch/rollback and compatible upgrade with live tasks | Required resources remain rooted; same tasks; runtime no Nix/store query or persisted store identity |
| D1 | 5 | Unified fzf plus CLI list/inspect/actions by ID | Same targets/state/freshness; no second selector; every new action has CLI parity |
| D2 | 5 | Remote lookup fails with unmatched query; select cached existing row | No local/replacement creation; cached explicitly labelled; reconnect exact ID |
| D3 | 5 | Sidebar/attention navigation through remote offline states | Host/freshness visible; provider status preserved; no stale attention presented as live |
| D4 | 5 | Background agent create/rename/Stop/Forget and explicit switch | View preserved except requested movement; typed failures preserve unrelated work |
| P1 | 6 | Representative output and graphics/resources vs full-frame baseline | Same authoritative visible state; dropped/out-of-order updates resynchronize |
| P2 | 6 | Slow/stalled receiver and reconnect | Bounded queue/memory; coherent full resync; no stale input effects |
| P3 | 7 | Read anchors during output/pruning/reflow under RTT/bandwidth constraints | Stable reading or explicit invalidation; bounded paging; measured end-to-end baseline comparison |

## Existing contracts affected

- EON-C8/C33: durable identity mapping, original reconstruction, final stopped
  view, and targeted Stop replace the identified old behavior only as the
  owning implementation slices land. Preserve existing tab/pane protection.
- EON-C10/C28: explicit durable targeting, CLI/JSON/failure and compatibility
  surfaces; avoid claiming 1.x frozen schema already includes these fields.
- EON-C11/C15/C23: remote attachment, build/protocol separation, supervisor
  reconstruction, window/Stop distinction; preserve existing generation and
  legacy eonbase behavior outside this feature.
- EON-C7: the remote Linux host requires no display; no implied additional
  architectures/platform proof. Local Mac work remains a separate proof.
- ORB-C3/C5/C13 and owning viewport/selection/resource contracts: replace
  current one-interactive-peer admission only after explicit approval;
  preserve host/terminal authority, per-client validation, bounded queues,
  independent local view behavior and trustworthy semantic input. Current
  per-run management lifecycle lease is not an exclusive human-device lease.
- EON-C31 and VEN-C8/C14/C21 boundaries: host/freshness/attachment presentation
  and attention consume authoritative Runtime state; no new terminal owner.
- EON-C2/C3 remain authoritative for one component graph and opaque component
  inputs. No second Nix compatibility graph or runtime Nix-store inspector.
