# 9. Security: who asks, and what may happen?

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

A **principal** means the identity/authority under which a request acts. An
**effect** means something the request may do: inspect, arrange, read, send,
close, run a process, or affect lifecycle. Runtime admits EONW requests by
principal and their possible effects before those effects occur. The classifier
lives beside the workspace state it examines
([admission.rs:1](../../runtime/crates/eon-runtime/src/admission.rs),
[admission.rs:25](../../runtime/crates/eon-runtime/src/admission.rs)).

A **socket** is an operating-system communication endpoint. The window's
workspace socket is an **owner endpoint**. A client reaching it
has the person's authority, with or without `EON_TERMINAL`. This environment
variable names the CLI's calling terminal. It routes and validates that asserted
terminal; it does not grant or reduce authority. The
**native principal** is the supervised presentation process, checked through
operating-system peer and process-start identity, with native-key exemptions
([docs/CONTRACTS.md:2465](../CONTRACTS.md),
[cli.rs:2789](../../runtime/crates/eon-runtime/src/cli.rs)).

A **native-key** exception lets the identified window process use its own
popup-key action, such as the Scrollback shortcut. **Governed** means a
terminal-content command subject to the `terminal_access` preference
([admission.rs:76](../../runtime/crates/eon-runtime/src/admission.rs),
[admission.rs:123](../../runtime/crates/eon-runtime/src/admission.rs),
[admission.rs:131](../../runtime/crates/eon-runtime/src/admission.rs)).

File permissions and admission answer different questions. Runtime checks an
owned mode-0600 socket on connection, helping separate local users. Its
`terminal_access` preference can refuse content commands. Neither establishes
isolation among unrestricted processes of the same user. A socket client can
hold other owner authority even when content commands are disabled
([control.rs:171](../../runtime/crates/eon-runtime/src/control.rs),
[control.rs:640](../../runtime/crates/eon-runtime/src/control.rs),
[docs/CONTRACTS.md:2474](../CONTRACTS.md),
[SECURITY.md:43](../../SECURITY.md)).

## Run it

From the repository root, in Nushell. Read the authority model without sending
a control request:

```nu
^nl -ba runtime/crates/eon-runtime/src/admission.rs | ^sed -n '14,23p'
^nl -ba runtime/crates/eon-runtime/src/admission.rs | ^sed -n '106,128p'
```

## What to look at

`Denying` has `#[cfg(test)]`: that deliberately restricted principal exists for
tests, not as a shipped per-agent grant system. The production guard checks
`required.governed`, `TerminalAccess::None` and the native-key exception. Do
not mistake a test mechanism for a delivered sandbox
([admission.rs:20](../../runtime/crates/eon-runtime/src/admission.rs),
[admission.rs:114](../../runtime/crates/eon-runtime/src/admission.rs)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [admission.rs:14-23](../../runtime/crates/eon-runtime/src/admission.rs):

- Lines 14–19 define Owner and Native. The comment explains how Native
  identifies the supervised presentation and adds the native Scrollback key.
- Lines 20–23 add Denying only when compiling tests. Tests can deny one
  effect to check that admission precedes it.

Then walk [admission.rs:106-128](../../runtime/crates/eon-runtime/src/admission.rs):

- Lines 106–113 define the admission call: principal, optional request,
  required effects and configured terminal-content access. Reserved Stop can
  have no versioned request.
- Lines 114–122 implement the test-only denial before proceeding.
- Lines 123–124 recognize a native-key request. The helper requires a
  callerless Toggle or Focus popup invocation at
  [admission.rs:131](../../runtime/crates/eon-runtime/src/admission.rs).
- Lines 125–128 refuse governed content access when disabled, except for
  that native key, then return success. The effects infrastructure and this
  preference check do not themselves grant different production permissions to
  individual agents.

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Can a sandboxed or compromised agent control the host through Eon?”

Yes, socket reachability can cross that boundary. The security policy says a
sandboxed agent must not reach Eon's or helper sockets: it can run host commands
through them even under `terminal_access = "none"`. Same-user unrestricted
programs have no isolation promise. Accepted beta risks include cross-agent
input and inherited launcher credentials; per-agent authorization remains
future work ([SECURITY.md:43](../../SECURITY.md)).

Admission makes requests and their effects explicit; it does not remove those
accepted risks. The security model describes an older audit snapshot plus
dated updates. For current authority, use the EON-C10 clause and code above,
not a historical finding as if no fix had landed
([docs/SECURITY-MODEL.md:3](../SECURITY-MODEL.md),
[docs/CONTRACTS.md:1426](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

Someone proposes `terminal_access = "none"` as their agent sandbox. Explain the
remaining owner-endpoint risk and distinguish routing, native identity and
OS-level socket reachability. Revisit the [worked example](examples/README.md).
<!-- owner-only:end -->

## Going deeper

- [Control authority, EON-C10, docs/CONTRACTS.md:2465](../CONTRACTS.md).
- [Accepted risks, SECURITY.md:35](../../SECURITY.md).
- [Dated threat model, docs/SECURITY-MODEL.md:1](../SECURITY-MODEL.md).
- Next: [packaging and proof](10-packaging-and-proof.md).
