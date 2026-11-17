# 4. Workspace model: what belongs where?

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

Eon uses the following product hierarchy. Runtime owns the live topology, the
arrangement of objects and their identities; Venus draws it. A tab groups work
around a launch directory. Its columns each contain a vertical stack of panes
with one expanded pane. A pane shows an Orbit terminal
([docs/ARCHITECTURE.md:38](../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:323](../ARCHITECTURE.md)).

| Object | Contains or refers to |
| --- | --- |
| Window | sessions, one shown at a time |
| Session | tabs |
| Tab | up to four columns, plus its popup terminals |
| Column / Stack | panes, one expanded |
| Pane | a persistent terminal |

The naming boundary defines the hierarchy; EONW carries the popup catalog and
tab-scoped popup terminals
([docs/ARCHITECTURE.md:34](../ARCHITECTURE.md),
[docs/ARCHITECTURE.md:106](../ARCHITECTURE.md)).

A **popup terminal** appears in an overlay such as the Agent popup, rather than
in a pane column. A **pending tab** is still at its directory-picker stage
([docs/USAGE.md:128](../USAGE.md), [docs/CONTRACTS.md:669](../CONTRACTS.md)).

A **snapshot** copies the owned model into data a client can read. It describes
objects and selected state; it is not a saved process. A separate registry
persists session identities and optional names, but stores no layout, process
ID or liveness. A queryable old registration does not prove recoverable work
([workspace.rs:1537](../../runtime/crates/eon-runtime/src/workspace.rs),
[docs/ARCHITECTURE.md:333](../ARCHITECTURE.md)).

## Run it

From the repository root, in Nushell:

```nu
^nl -ba runtime/crates/eon-runtime/src/workspace.rs | ^sed -n '1537,1583p'
^nl -ba docs/ARCHITECTURE.md | ^sed -n '333,357p'
```

## What to look at

In the snapshot, find the distinction between `workspace`, `active_workspace`,
`active_tab`, `selected_pane` and `selected_popup`. In the registry description,
find “no membership, layout, PID or liveness.” A field named `persistent_id`
does not turn all the neighboring fields into durable state
([workspace.rs:1544](../../runtime/crates/eon-runtime/src/workspace.rs),
[docs/ARCHITECTURE.md:349](../ARCHITECTURE.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [workspace.rs:1537-1583](../../runtime/crates/eon-runtime/src/workspace.rs):

- Lines 1537–1538 construct a `Snapshot`, a Rust record of named fields.
- Lines 1539–1543 leave focus and build/generation strings unset here. The
  comment names another boundary that stamps identity. Empty strings at this
  internal stage are not evidence that the public response lacks identity.
- Lines 1544–1554 copy the session's ID, persistent registration facts, name
  and tab count. This method begins with one workspace entry. The enclosing
  `WorkspaceSet::snapshot_in` projects the window's session summaries at
  [workspace.rs:589](../../runtime/crates/eon-runtime/src/workspace.rs).
- Lines 1555–1556 copy the active tab and popup geometry.
- Lines 1557–1566 turn the popup catalog into entries with an ID, label and
  shortcut. `iter().map(...).collect()` means “make one output record per
  input record.”
- Lines 1567–1573 begin projecting tabs: ID, launch-directory bytes and
  whether the tab is pending. The directory field describes Runtime policy,
  not an inference from a shell prompt
  ([docs/ARCHITECTURE.md:326](../ARCHITECTURE.md)).
- Lines 1574–1577 select the expanded pane of the selected column, if a
  column is selected. Lines 1578–1583 copy popup selection and begin projecting
  columns; the excerpt stops inside that nested construction.

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Which workspace state survives which failure?”

Detaching and returning to the same supervisor retains its workspace. After
supervisor loss, same-boot recovery constructs a new synthetic session/tab from
surviving runs rather than promising the old topology. The registry records
identity and names, independently of live membership. These are three distinct
forms of continuity, and the recovery contract's installed proof gap still
applies ([docs/ARCHITECTURE.md:114](../ARCHITECTURE.md),
[supervisor.rs:1614](../../runtime/crates/eon-runtime/src/supervisor.rs),
[docs/ARCHITECTURE.md:349](../ARCHITECTURE.md),
[docs/CONTRACTS.md:3595](../CONTRACTS.md),
[docs/CONTRACTS.md:3612](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

Draw one window with two sessions and a tab containing two columns. Mark one
terminal and one persistent session ID. Explain why the ID alone cannot restore
that drawing. Try the [worked example](examples/README.md).
<!-- owner-only:end -->

## Going deeper

- [Workspace ownership, docs/ARCHITECTURE.md:323](../ARCHITECTURE.md).
- [Durable workspace, EON-C8, docs/CONTRACTS.md:558](../CONTRACTS.md).
- [Several sessions, EON-C33, docs/CONTRACTS.md:5763](../CONTRACTS.md).
- Next: [the protocol and CLI](05-workspace-protocol.md).
