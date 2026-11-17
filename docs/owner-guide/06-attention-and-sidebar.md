# 6. Attention: reported state becomes a useful signal

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

With several agents running, you need to know who needs input and who finished.
Runtime's attention owner polls program status off the request path. Orbit
owns the status records; Runtime derives what has finished, what needs input
and what the person has seen. The sidebar groups reporting terminals by tab
([attention.rs:1](../../runtime/crates/eon-runtime/src/attention.rs),
[docs/USAGE.md:928](../USAGE.md)).

An agent that supports the Program Status Protocol writes a structured
**OSC 7501** report through its terminal output, including a state such as
`blocked`. OSC is a terminal control-message family. Orbit validates the
report and stores the record; Runtime reads it through Orbit's
management interface. Ordinary English output does not set Blocked
([orbit/docs/CONTRACTS.md:1062](../../orbit/docs/CONTRACTS.md),
[orbit/docs/CONTRACTS.md:1085](../../orbit/docs/CONTRACTS.md),
[orbit/docs/CONTRACTS.md:1101](../../orbit/docs/CONTRACTS.md),
[orbit/docs/BEHAVIOR.md:1120](../../orbit/docs/BEHAVIOR.md)).

The program's reported state and the displayed attention state differ. A
reported `Blocked` becomes NeedsInput. An unseen error becomes Error. Done or
Idle with background processes becomes Background. An unseen finish becomes
Finished when background work does not mask it. These are explicit cases in
one function, rather than a promise that Eon understands an agent's prose
([attention.rs:145](../../runtime/crates/eon-runtime/src/attention.rs)).

A **raise** here records a change that may need attention, including whether
the person has seen it. Keeping change time separate from poll time lets the
owner retain the event's time while observing it again
([attention.rs:128](../../runtime/crates/eon-runtime/src/attention.rs),
[attention.rs:139](../../runtime/crates/eon-runtime/src/attention.rs)).

## Run it

From the repository root, in Nushell:

```nu
^nl -ba runtime/crates/eon-runtime/src/attention.rs | ^sed -n '137,163p'
^nl -ba docs/USAGE.md | ^sed -n '928,967p'
```

## What to look at

Read the `match` cases in order. The Background case precedes the Finished
case. In the user documentation, find `status stale` and `status unavailable`:
absence of a fresh report is a condition the UI describes, not confirmation
that the agent has stopped ([attention.rs:152](../../runtime/crates/eon-runtime/src/attention.rs),
[docs/USAGE.md:957](../USAGE.md)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [attention.rs:137-163](../../runtime/crates/eon-runtime/src/attention.rs):

- Lines 137–143 define the stored record: the program's report, the first
  observation time for its change, an optional raise and notification state.
- Lines 145–150 introduce normalization: take a record and a fact about
  background processes, then return an attention state with its ordering time.
- Line 151 keeps a raise only when it is unseen. `filter` removes the other
  case; it does not modify the stored record.
- Lines 152–154 prioritize blocked input and an unseen error.
- Lines 155–157 show Background for Done or Idle when background work remains.
- Line 158 shows Finished for Done or Idle with an unseen raise after the
  earlier cases have failed.
- Lines 159–163 keep Working, then default to Idle. Each case also returns a
  timestamp. The caller uses normalized state when choosing the most urgent
  record at [attention.rs:187](../../runtime/crates/eon-runtime/src/attention.rs).

The Background ordering is worth explaining aloud. “The agent reported done”
does not imply “the processes associated with its terminal are finished.” The
user docs describe Linux-only measured helper exemptions and a limit: work inside
an exempt REPL without a child process cannot be distinguished by that observation.
EON-C31 labels this helper interpretation Candidate, awaiting installed proof
([docs/USAGE.md:950](../USAGE.md), [docs/CONTRACTS.md:5007](../CONTRACTS.md)).

</details>
<!-- under-the-hood:end -->

## The hardest question

> “How do you know the agent needs help rather than guess from its output?”

Point to the reported states and the normalization function. The sidebar can
display the needs-input question and label stale or unavailable reports. It
does not certify that an agent's claimed success is correct. Program reports,
titles and attention text remain untrusted data, and background-process
observation has named limits. The Linux helper interpretation remains Candidate,
awaiting installed proof
([docs/USAGE.md:957](../USAGE.md),
[SECURITY.md:49](../../SECURITY.md),
[docs/USAGE.md:950](../USAGE.md), [docs/CONTRACTS.md:5007](../CONTRACTS.md)).

<!-- owner-only:start -->
## Explain it back (owner only)

Walk through three reports: Blocked; Done with background work; Done with an
unseen raise and no background work. Name the displayed state and explain why
none proves the task's result is correct.
<!-- owner-only:end -->

## Going deeper

- [Agent attention, EON-C31, docs/CONTRACTS.md:5005](../CONTRACTS.md).
- [Sidebar behavior, docs/USAGE.md:928](../USAGE.md).
- Next: [managed agents](07-managed-agents.md).
