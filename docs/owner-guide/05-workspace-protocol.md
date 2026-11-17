# 5. Workspace protocol: one action, two ways to ask

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

Eon's rule for humans and agents requires a CLI path for workspace actions and
data for visible workspace state, subject to named exceptions. That gives an
agent a way to act through the workspace model rather than simulate mouse
movement. Venus-local view state, such as hover and selection, has exceptions
([AGENTS.md:109](../../AGENTS.md),
[docs/AGENT-RULES.md:246](../AGENT-RULES.md)).

EONW is the **protocol**, the message agreement at that workspace boundary.
Its owner crate defines versioned values, validation and encoding. Runtime
owns the live model; Orbit owns its own terminal transport. The current EONW
codec, the code that converts messages to and from bytes, names version 47 and
a 12-byte header. Those numbers identify a wire
format, not a count of product contracts
([docs/ARCHITECTURE.md:359](../ARCHITECTURE.md),
[v47.rs:1](../../runtime/crates/eon-workspace-protocol/src/v47.rs),
[v47.rs:28](../../runtime/crates/eon-workspace-protocol/src/v47.rs)).

For one concrete trace, follow `CreateTab`. In the parity map, that EONW action
maps to the CLI words `tab create`. A test parses the words back and checks
that it gets the same action. This is how you move from “we have a rule” to
“here is a check that can catch one way of violating it”
([cli/parity.rs:64](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[cli/parity.rs:435](../../runtime/crates/eon-runtime/src/cli/parity.rs)).

## Run it

From the repository root, in Nushell. These commands read the mapping and its
test; they do not create a tab:

```nu
^nl -ba runtime/crates/eon-runtime/src/cli/parity.rs | ^sed -n '59,72p'
^nl -ba runtime/crates/eon-runtime/src/cli/parity.rs | ^sed -n '433,454p'
```

## What to look at

Find `CreateTab => words(&["tab", "create"])`. Then find `assert_eq!` comparing
the parsed command with `Ok((action, false))`. The enclosing test is
`every_eonw_action_has_a_cli_path_or_a_named_exception`, defined at
[cli/parity.rs:213](../../runtime/crates/eon-runtime/src/cli/parity.rs).
This is source evidence for a test mechanism, not a report that it passed today.

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [cli/parity.rs:59-72](../../runtime/crates/eon-runtime/src/cli/parity.rs):

- Lines 59–63 map runtime inspection and presentation actions to lifecycle
  commands. These are a different check category from directly parsed actions.
- Line 64 maps `CreateTab` to two words. Lines 65–72 assemble the arguments
  for creating a session, including its optional name and focus choice.

For each sample action, the test obtains its CLI path at
[cli/parity.rs:425](../../runtime/crates/eon-runtime/src/cli/parity.rs).
Walk the checks at [cli/parity.rs:433-454](../../runtime/crates/eon-runtime/src/cli/parity.rs):

- Lines 433–443 handle directly parsed commands: check the command gate,
  parse targeting flags, then close arguments or ordinary control arguments.
- Lines 444–449 demand equality with the expected action and human-output
  mode (`false` for JSON). Wrong mapping or parsing can fail this assertion.
- Lines 450–454 check only a help entry for the separate lifecycle category.

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Does a contract ID enforce anything, and can an agent do what the window can?”

The ID locates an obligation. The implementation and checks enforce particular
parts. EON-C10 owns typed workspace control, and the parity map checks a
CLI/action relationship. Its exhaustive match makes a new unmapped action a
test-build problem. The test calls separately dispatched commands such as
`window attach` “lifecycle entries.” It checks their help entries rather than
parsing them back to actions, so their mappings still need review.
Four window actions still have no CLI path here:
creating a column, focusing an attention target, and next/previous attention.
The map records these as implementation gaps. Separate Venus-local view-state
exceptions belong to the Humans and Agents rule
([docs/CONTRACTS.md:1424](../CONTRACTS.md),
[cli/parity.rs:1](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[cli/parity.rs:13](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[cli/parity.rs:129](../../runtime/crates/eon-runtime/src/cli/parity.rs),
[docs/AGENT-RULES.md:246](../AGENT-RULES.md)).

Do not infer runtime success, safe authorization or complete GUI correctness
from a parser-equality assertion. Chapter 9 covers who may ask; chapter 10
distinguishes this source reading from execution and installed proof.

<!-- owner-only:start -->
## Explain it back (owner only)

Trace `CreateTab` from a typed action to CLI words and back through the test.
Give one reason that this check could pass while an installed interaction fails.
<!-- owner-only:end -->

## Going deeper

- [Typed control, EON-C10, docs/CONTRACTS.md:1424](../CONTRACTS.md).
- [Parity categories, cli/parity.rs:10](../../runtime/crates/eon-runtime/src/cli/parity.rs).
- Next: [attention](06-attention-and-sidebar.md).
