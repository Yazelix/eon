# Writing Bend 2

Practices for Bend 2 as pinned here (234cb9b9, release 2.0.37), from Bend's own
guides, its runtime paper and issue tracker, its author's posts, outside ports,
and Orbit's own failures. Each rule names its source; "here" means Orbit,
including its development in the bend-orbit repository, whose issues
`borb-orbit-in-bend-lj8.N` the rules cite as lj8.N.

## Treat Bend as a code factory

Bend is built for a loop in which a human writes the specification and a
machine writes the code. Taelin's framing: humans "write specs - i.e., what we
want, as *precise types* - and the AI does the coding, and then *proves that it
is correct*" ([post][brand]). Orbit splits the work the same way:
`LAWS.bend` holds the human's claims, `PROOF.bend` the AI's proofs, and
`bend PROOF.bend` gates every change. Bend trades speed of writing for
correctness on purpose: "it is *always* a good trade to make the model slower
yet more correct" ([post][trade]).

What follows from that:

- Spend human attention on `LAWS.bend`. Laws "only protect what you remember
  to write" (Taelin, [HN][hn]), and are read instead of the code, since they are
  "substantially smaller than the codebase". A single underspecified law let
  the demo's AI satisfy "you can't win" by making the flag unenterable or by
  disabling movement ([HN][hn]).
- Keep agents from editing laws to fit their code: review law diffs as
  specification changes and run the proof gate in CI ([HN][hn]).
- Generate repetitive Bend instead of typing it. The pixel-fixer port had its
  agent write a kernel generator and a topological sorter to emit Bend
  ([port][pixel]). Inside Bend, `~` template parameters are compile-time code
  generation ([GUIDE.md][guide]). Here, a state-monad `Parse` with `do`
  notation replaced hand-written continuation chains in bend-orbit's ORBF
  encoder experiment (`experiments/orbf/encode.bend`, not part of Orbit).
- Check which model your agents run. Taelin found Claude Code subagents
  silently routed to an older model while working on Bend ([post][routing]).

## Checker rules that shape code

Bend checks in one pass and infers almost nothing. These rules decide how a
file must be laid out ([GUIDE.md][guide], [WONTFIX.txt][wontfix], here):

- Definitions are checked in file order: a callee must appear above its caller,
  and two defs cannot call each other. Write the callee first; turn a mutual
  pair into one def with a selector argument. An `@unsafe` def may recurse
  freely and call a def below it, but the verdict then lists it and every def
  that depends on it.
- A closure is affine, and an `@unsafe` def that marks a function parameter `+`
  is not checked for it. At run time a closure built only from constants, such
  as a top-level def applied to constant arguments, is static and copies
  freely, but one that captures a run-time value lives on the heap and cannot be
  counted: copying it stops the program with `runtime fail-stop` (here). Share
  a top-level def or a constant closure instead.
- Termination reads arguments left to right: each must pass unchanged until one
  shrinks. Put the shrinking argument first. A loop bounded by the outside world
  counts down a `Nat` fuel argument.
- A `match`, and a destructuring let such as `(a, b) = f(x)`, take only a
  parameter or a pattern-bound variable, never a computed value. Pass the value
  to a helper def that matches its parameter. In a `do` block, this splits each
  decision into call, unpack, `IO.pass` and match (here).
- Binders are matched in declaration order, and a let may not precede a match on
  a parameter. A variable a pattern binds takes its scrutinee's place in that
  order: after matching a later parameter, the head of an earlier list can no
  longer be matched. Match the list, then its head, then the later parameter,
  or both at once (`match first pace:`) (here, LAWS.bend `fold`).
- Type names share Base's namespace: `Event` is taken (here). Constructor names
  must not collide with Base's (`None`, `Some`, `Con`, `U32`, `Key`, `Text`,
  `Scroll`, ...), even inside an imported module (here).
- `Maybe<(U32 & U32)>`: a compound type argument takes parentheses (#1110).
- `@+x: A -> B` is not `A -> B`: pass `x => f(x)` where a plain function is
  expected (#1111).
- `++` joins only `String`s; join lists with `List.append(&2, T, xs, ys)` and
  build them with `[a, b]` and `h <> t` ([GUIDE.md][guide]).
- A def returning `Type` is a type alias: `def Decisions() -> Type:` with body
  `List<&2, Decision>` is written `Decisions()` in signatures and pairs, as
  Base's `IO(A)` is (here).
  A shared `+` parameter must have a `Data` type, so it cannot be written with
  the alias: write `+decisions: List<&2, Boundary.Decision>` (here).
- A fallback arm (`case _:`) does not refine a dependent goal; write the
  constructor arms (#1091). A shadowed arm is dead code the checker skips.
- No common-subexpression elimination: two calls of `f(p)` run twice. Bind once
  (#812).
- `bend file.bend` runs a pure `main` lazily; compiled binaries are strict
  (#775). Measure compiled code only.
- `--check-only` on a program with native effects always prints
  `SOME PROOFS FAIL`, listing the foreign and unsafe defs; `PROOF.bend` prints
  `ALL PROOFS CHECK` when every law holds (here).

## Performance

Bend compiles to one C file. Every def becomes a segment of one worklist
function, with no C stack ([BendRT][bendrt] section 3). Its cost model is
readable, and these are the parts that matter.

### Flat loops, not non-tail recursion

A call in tail position is a jump; a def with no fork, closure apply or
non-tail self call compiles to one native loop. A non-tail self call becomes a
heap continuation per call ([SHADERS.md][shaders]). Here, a prepend-style
encoder (`put_cell(c, put_cells(rest, out))`) compiled to three continuation
segments per cell. Rewritten as tail loops threading `(array, index)`, it ran
about five times faster.

### Arrays for bulk data, lists for walks

An `Array<T>` is one flat block, read and written in place, with one owner
([BendRT][bendrt] section 4.3). A `List<U32>` costs a heap cell per element.

| Workload | List | Array | Source |
| --- | ---: | ---: | --- |
| ORBF frame, 80x24 (50 KB) | 0.70 ms | 0.144 ms | here |
| ORBF frame, 300x100 (760 KB) | 11.8 ms | 3.3 ms | here |
| pixel-fixer, one image | 3.6 min | 4.5 s | [port][pixel] |
| 384 MiB read and summed | 1.52 s | C `pread`: 57 ms | [#1573][i1573] |

Byte effects (`File.read_bytes`, `TCP.recv_bytes`, `File.write_bytes`) produce
or consume a `List<U32>` with one cell per byte, and `String` is a list of
characters. An effect that returns a `String` Bend only inspects for emptiness
still builds every cell (here, lj8.37). For bulk input, a foreign effect that
fills an `Array` cut model loading from 72 s to 9 s ([#1573][i1573]).

An `Array` cannot cross a fork, because it has one owner. Parallel stages build
lists or per-branch arrays and join them ([SHADERS.md][shaders]).

### Sharing has a whole-program cost

A type is *hot* when any binder of it is used more than once anywhere in the
program. A hot type's constructors seal every stored field with a reference
count at build time, and matches on it copy instead of reading in place
([BendRT][bendrt] section 3.1). One `+` on a `List<&2, U32>` in an unrelated
function made a loop that shares nothing run 2x slower ([#1559][i1559]). Here, a
length pass over a shared `+payload` list cost more than the encoding around
it.

- Use `+` only where a value is really used twice, and prefer passing a value
  to a function that only reads it: borrow inference then lends it with no
  count ([BendRT][bendrt], [SHADERS.md][shaders]).
- A def that returns its argument, or stores it in a constructor, owns it;
  refolding a matched `+` record at a call shares every field.
- Read the emitted C (`-o x.c`): `term_peek` is a borrow; `term_keep`,
  `ctr_take` and `rfc_seal` are counts ([SHADERS.md][shaders]). To find where a
  bare `runtime fail-stop` comes from, have the emitted `err_post` print its
  code and a `backtrace()`, and build with `-g` (here).

### Smaller rules

- A `match` on dense `U32` or `Nat` literals compiles to a lookup table
  (changelog 2.0.19); a sparse `U32` literal match is a bit tree with the
  default arm copied per branch ([SHADERS.md][shaders]). Typed pickers beat
  the generic `Bool.pick`.
- A literal `Nat` fuel such as `60n` unrolls the recursion; pass
  `U32.to_nat(60)` instead (#886).
- Wide records spill through the stack: build them where they are used
  ([SHADERS.md][shaders]).
- A parallel let `a b = f(x) g(y)` promises independent calls of similar cost;
  the scheduler assumes balance. Use one parallel let per def
  ([GUIDE.md][guide], [SHADERS.md][shaders]).
- `Nat` is a 48-bit immediate (fail-stop past 2^48-1) and there is no U64;
  carry 64-bit values as two `U32` halves (here). Silent `U32` wraparound caused
  two of the pixel-fixer's three bugs ([port][pixel]).

### Measuring

- `IO.now` has millisecond resolution: time batches of iterations and divide.
  Check that time scales linearly with iterations (here).
- Repeating the same allocation-heavy work in one process slows it 3x to 17x
  as free lists lose address order ([#1007][i1007]). Use fresh processes or
  interleave runs rather than trusting a long in-process loop.
- Build binaries as `bend` does (`bun bend2/main.ts x.bend -o x.c`, then
  `clang -std=c11 -O3 x.c -lpthread -lm`), and pin `--threads`.

## Effects and the C boundary

Custom effects are a def whose body imports a `.c` and a `.js` file
([EFFECTS.md][effects]). Here, avoiding repository C moved every native
mechanism into a Rust host process: the Bend worker uses only Base's `File`
effects on two inherited pipes (`/dev/fd/3` and `/dev/fd/4`), at about 15 us
per round trip in a first measurement, plus Bend's own parsing ([HOST.md](HOST.md)).
Rules for writing custom effects, learned before that move:

- The C interface has no ABI promise: rebuild effects on every Bend update and
  pin the Bend revision ([EFFECTS.md][effects]). Here, one update required
  porting the whole adapter.
- A handle owns its descriptor. An effect that takes a handle borrows the fd
  and hands back the same handle; only a close effect closes. User handle types
  are not allowed yet, so reuse Base's `File` or `Socket` laws (#825).
- Block with `io_wait_on(w, fd, POLLIN|POLLOUT, deadline, more)` or
  `io_work`. An effect that can face a steady backlog parks before its first
  try so timers still run.
- Bend's loop is sequential between parallel calls: an effect that parks on one
  client stalls every other source the loop serves. Here, an interactive send
  that waited for a slow client stopped PTY reads and observers until Rust
  queued output and reported pressure instead (lj8.33).
- The runtime defines short C names (`u32`, `lock`); include headers with care
  ([Akita][akita]). Extra libraries link through a `$CC` wrapper.

## Proofs

- Base has almost no arithmetic lemmas (`U32.add_comm` and few others): budget
  for writing your own, and keep laws small ([HN][hn], [Akita][akita]).
- Prove a small pure policy, not the effectful program. Akita proved a 30-line
  guard around an unproven allocator; here, `src/lifecycle.bend` and
  `src/vertical.bend` are pure modules the running loop calls and
  `PROOF.bend` proves.
- An induction over a list must take the list as its first parameter.
- An equality cannot be matched. To split on a computed `Bool`, take it as a
  parameter of a helper whose result is a function of its equation,
  `{f(x) == b : Bool} -> Goal(b)`: matching `b` refines both, and the caller
  passes `f(x)` and `{==}`. Rewrite with `%e : P` and `Equal.sym` (here,
  `src/revision.bend`).
- `U32` is `U32{Word(32n)}`, bits from the least significant, so a lemma
  about a `U32` operation is an induction over `Word(n)`; a literal such as
  `4294967295` is convertible to the word it denotes. A proof binder used
  twice needs `+`, including a successor's predecessor: `case 1n+(+p):`
  (here).
- A proof about a whole program needs its pure part in its own module: a
  proof file that imports a module with `@unsafe` or IO defs prints
  `SOME PROOFS FAIL` even when no law uses them (here, `src/session.bend`
  holds `step` apart from `src/main.bend`'s loop).
- A destructuring let, `(a, b) = pair`, does not reduce until `pair` is a
  tuple, and a call stays folded until every value it matches on is a
  constructor. So a proof about a composed function splits cases until the
  goal computes. The splits must follow parameter order, since a binder
  cannot be matched after a later one: put the parameters to split first, and
  give a lemma its variables as separate parameters in that order. Generate
  the splits: `tools/step-proof.rs` asks Bend which variable a failing leaf's
  stuck call matches on, about 7,800 leaves for `step` (here, ueg.8).
- A second law over the same composed function can start from the first law's
  splits, which `step` needs anyway: `step-proof view` seeds its
  lemmas from `proof/step.bend`, and the search added about 1,900 leaves to
  the lifecycle law's 12,300 in about two minutes (here, ueg.9). The
  management law added about 1,700, and the whole proof with three such laws
  checks in about 11 seconds and 2.1 GB (ueg.10).
- A law can say only that `step` may do something, as the lifecycle law lets a
  management Stop begin the stop. A sibling law that must pin it down needs a
  sensitivity check the first law accepts: here `step` answering a leased Stop
  with nothing passes the lifecycle law and breaks the management law
  (ueg.10).
- Each flag that decides whether a decision is in the list splits every leaf
  that reaches it, because the automaton reads the list's shape. The
  observer's `metadata_changed` took `step` from about 7,800 leaves to 12,300
  and the check from about 2.3 to 3.4 seconds; a flag carried only as a
  decision's field, such as `metadata_fresh`, splits nothing (here, ueg.11).
  Fact `Resources` first answered with two decisions, `RetireResources` and
  `StartResource`, each in the list or not by its flags, under a publication
  like output's: its lemmas took 3,900 to 5,000 leaves per law, the whole
  proof went from about 2.3 GB to 5.1 GB, and the lifecycle search took 30
  minutes at `JOBS=4`, since each split rechecks the whole lemma. One
  decision, `Transfer`, that is always there and carries the choice as its
  fields `start` and `retire`, halved that: 2,000 to 2,400 leaves per law,
  2.7 GB, and an 11-minute lifecycle search. The law over the policy then
  holds by computation alone. The tool seeds a fact new to a law's file from
  `proof/step.bend`, as it seeds a new law: so seeded, the view search took
  3 minutes and the management search 2 (here, orb-j2i.7).
- A value computed from a variable that cannot be split, such as a revision
  comparison over symbolic `U32`s, blocks every split below it. Compute it
  once at the top and pass it down as a `Bool`, so the lemma can take it as a
  parameter (here, `future_of`).
- A field an automaton tracks constrains nothing unless some answer depends on
  it. A per-fact lemma that equates the automaton with the state `step`
  reaches is stronger than the law, so it can fail for a change the law
  accepts. Here the view law's revision was checked only by its lemmas until
  the automaton's own revision decided which requests are from the future
  (ueg.9). Confirm a sensitivity check breaks the law on a concrete trace.
- After proving a law, break the code on purpose and confirm the proof fails;
  a law that survives every mutation constrains nothing (here, lj8.26). Check
  that it fails because the law is broken, not because a case split stopped
  reducing.
- `--verdict` rechecks with the Lean-proven kernel. F32 is axiomatic: no law
  can say anything about floating-point results.

## Sources

- Bend at 234cb9b9: [GUIDE.md][guide], [SHADERS.md][shaders],
  [EFFECTS.md][effects], [WONTFIX.txt][wontfix], [CHANGELOG.md][changelog],
  the BendRT paper (`bend2/docs/BendRT/main.typ`).
- Issues: [#1559][i1559] (hot types), [#1007][i1007] (repeated work),
  [#1573][i1573] (byte lists).
- Taelin's posts: [spec-first branding][brand], [correctness over speed][trade],
  [subagent routing][routing], [launch notes][launch].
- Reports: [pixel-fixer port][pixel], [Akita on Rails][akita],
  [Hacker News thread][hn].
- Here: [BEHAVIOR.md](BEHAVIOR.md), bend-orbit's `experiments/orbf`, and the
  lj8 beads.

[guide]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/guide/GUIDE.md
[shaders]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/guide/SHADERS.md
[effects]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/guide/EFFECTS.md
[wontfix]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/WONTFIX.txt
[changelog]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/CHANGELOG.md
[bendrt]: https://github.com/bendlang/bend/blob/234cb9b950800f20b15feafcb4ce2f43e76e7fae/bend2/docs/BendRT/main.typ
[i1559]: https://github.com/bendlang/bend/issues/1559
[i1007]: https://github.com/bendlang/bend/issues/1007
[i1573]: https://github.com/bendlang/bend/issues/1573
[brand]: https://x.com/VictorTaelin/status/2023405334677913720
[trade]: https://x.com/VictorTaelin/status/1973026035579879499
[routing]: https://x.com/VictorTaelin/status/2103309609431601409
[launch]: https://x.com/VictorTaelin/status/2100374221671051472
[pixel]: https://x.com/RealAstropulse/status/2101132976704622908
[akita]: https://akitaonrails.com/en/2026/09/19/new-ai-language-just-released-bend-2/
[hn]: https://news.ycombinator.com/item?id=49746163
