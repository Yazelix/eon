# Contract Index

Contract IDs give cross-repository decisions stable names. `Planned` means the
repository records intent without implementation evidence. `Proven` requires an
exact check, component revisions, platform, and artifact.

| ID | Contract | Owner | Status | Proof |
|---|---|---|---|---|
| AST-C1 | Astra launches one compatible component set and reports every exact component revision | Astra | Planned | None |
| AST-C2 | One versioned manifest defines the component graph for every distribution channel | Astra | Planned | None |
| AST-C3 | Astra receives explicit component paths, treats Nix store paths as opaque launch inputs, and invokes no Nix evaluator during normal use | Astra | Planned | None |
| AST-C4 | Astra preserves child ownership and adds no duplicate terminal, rendering, editor, file-manager, or configuration state | Astra | Planned | None |
| AST-C5 | A user can install, upgrade, inspect, and remove a direct Astra bundle without replacing an existing unrelated toolchain | Astra | Planned | None |
| AST-C6 | The Nix alpha and later distribution channels consume the same accepted component graph without changing runtime semantics | Astra | Planned | None |
| AST-C7 | Release design accounts for Linux and native signed and notarized macOS artifacts | Astra and Venus | Planned | None |

## Proof record

When a bead proves a contract, update its row with:

- the test or dogfood command;
- exact Git revisions and component manifest;
- operating system and architecture;
- produced artifact and checksum;
- limitations that remain inside the contract.

Do not change a row to `Proven` from a design review, mock, or unexecuted test.

## Change rule

A contract change needs an explicit bead. Record the user-visible consequence,
affected owners, compatibility decision, test change, and rejected alternative.
If another repository owns the contract, land and verify the owner change before
Astra consumes it.
