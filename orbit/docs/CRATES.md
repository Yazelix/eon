# Orbit crate decisions

This is the durable index of direct and architecture-shaping crate decisions.
The [reference map](REFERENCES.md) records projects worth studying; appearance
there does not approve a dependency. The [root `AGENTS.md`](../../AGENTS.md)
defines the dependency gate, and Beads retain the full candidate evidence.

## Current decisions

### Authoritative terminal semantics

- **Selected shape:** `libghostty-vt` 0.2.2, with `libghostty-vt-sys` 0.2.2 and pinned
  Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083`
- **Status:** Selected for the experiment
- **Credible alternatives:** Formatter reconstruction with the same crate; a binding
  extension or fork; replacement by `rio-vt`
- **Why:** The published crate supplies the chosen sole terminal authority and state-aware
  input. Its Formatter cannot provide exact client reconstruction, so Orbit keeps authority
  server-side and uses its public presentation surface.
  Vendored builds explicitly select Zig's `baseline` CPU by default;
  `LIBGHOSTTY_VT_SYS_CPU` permits an intentional deployment-specific override.
  Release 0.2.2 preserves the pinned engine and generated bindings, adds wrapper
  safety and Windows linking fixes, and leaves the dependency graph unchanged.
  Adopting the upstream release avoids an owned build-script patch; Rust-only
  CPU flags do not control this Zig build. No owned runtime logic is added.
- **Evidence:** `orb-bi4.1` and its immutable negative corpus at
  `ac0a291ebe7fcebe4d7cace914b89363856c4903`; accepted `orb-bi4.3` proof
  `e4fde443e625180d7332eff4dff366f64bee30a8`; portable build adoption
  `427cf2d074cd1db905a2cc2602a5475065d1c790` ([PR #1](https://github.com/Yazelix/eon-sessions/pull/1)),
  with downstream acceptance tracked by Eon `eon-portable-orbit-cpu-omxq`.

### Platform-specific PTY and local-runtime mechanics

- **Selected shape:** One concrete `src/platform.rs` seam around direct `libc` 0.2.189 calls
- **Status:** Selected and proved for ordinary and managed x86_64 Linux and Apple
  Silicon macOS Sessions
- **Credible alternatives:** Handwritten C ABI with no crate; `pty-process` 0.5.3; `nix`
  0.31.3 with narrow `fs`, `poll`, `process`, `signal`, and `term` features; `portable-pty`
  0.9.0; `rustix` 1.1.4 with `rustix-openpty` 0.2.0
- **Why:** The seam owns PTY setup, process groups, descriptors, polling, signals, socket
  paths and permissions, and platform EOF classification while the owner loop sees concrete
  platform-neutral outcomes. Ghostty confirms the physical PTY split; portable-pty's trait
  model and eleven-package normal dependency graph are broader than Orbit needs. The
  measured `nix` shape adds `nix` and `cfg-if` plus a build dependency without reducing net
  owned LOC; the rustix shape adds `rustix-openpty`, `rustix`, and `linux-raw-sys`, with
  more packages needed for signals. At the accepted PTY candidate, the direct shape kept
  Cargo unchanged and occupied 336 lines in `src/platform.rs`; `src/main.rs` was 579 lines,
  and total owned Rust was 1,554 lines. A focused `pty-process` scratch candidate passed all
  13 tests and the canonical checks, removed 44 owned lines, and added `pty-process`,
  `rustix`, and `linux-raw-sys` to the normal Linux graph. The accepted proof retains the
  no-manifest candidate; adopting `pty-process` remains a separate user-approved crate gate.
  Native Apple Silicon proof confirms Darwin's zero-length master-read closure maps to the
  existing platform-neutral `Closed` outcome without a second PTY implementation. The same
  seam uses Darwin `getpeereid` and `proc_pidinfo(PROC_PIDTBSDINFO)` for the existing
  management peer and opaque process-start identities without changing management policy or
  adding a dependency.
- **Evidence:** Retroactive crate gate in `orb-bi4.2`; exact Apple, Ghostty, portable-pty,
  and libc sources plus the portability disposition are recorded in `orb-pmp`;
  post-candidate `pty-process` evidence is recorded there for a future decision; accepted
  ordinary ORB-C14 proof `6c90ef467abb520a2226a8674a7d0d7fb6f54099`; accepted native
  management proof `0233f4d34b294a50c5bc7f373859cf4ec04d2414` in
  [native run 34654742795](https://github.com/Yazelix/eon-sessions/actions/runs/34654742795)

### Linux PTY Session shutdown

- **Selected shape:** Existing direct `libc` 0.2.189 process-group signals and direct-child
  reaping in `src/platform.rs`
- **Status:** Selected replacement candidate in `orb-portable-pty-shutdown-ke1`; no
  manifest, feature, normal-graph, helper-service, privilege, namespace, or build change
- **Credible alternatives:** Retain mandatory cgroup v2; make cgroup cleanup optional; SID
  or `/proc` traversal with subreaping and pidfds; Linux 6.9+ group pidfds; privileged
  namespace or systemd supervision; a new process-management crate
- **Why:** Orbit already owns the initial PTY process group, current foreground group,
  direct child, signal seam, and bounded wait. Reusing those values removes 199 net
  production Rust lines and 171 net lifecycle-test lines from the current candidate. It
  gracefully targets the terminal-owned shell and foreground job, then force-targets process
  groups only while the direct child remains unreaped: no delayed SIGKILL uses a cached
  foreground-group number after the shell exits. A deliberately detached process or
  SIGHUP-ignoring foreground job may survive; the contract says so instead of scanning or
  guessing ownership. Optional cgroups retain nearly all lifecycle and proof cost while
  making Stop environment-dependent. PID/SID scans, subreaping, and ordinary pidfds still do
  not atomically discover every descendant; group pidfds would raise the minimum Linux
  kernel to 6.9 for a stronger contract the user did not choose. The accepted earlier cgroup
  proof remains evidence for the stronger retired behavior and its delegation cost, not
  current authority.
- **Evidence:** Replacement decision and reference gate in `orb-portable-pty-shutdown-ke1`;
  historical stronger proof `7f067b30e97d0b4787a7c6c0bbe3dd8a80a61c2c`

### Owner-loop scheduling and concurrency

- **Selected shape:** Owned synchronous poll/event loop at Orbit
  `e4fde443e625180d7332eff4dff366f64bee30a8`
- **Status:** Selected for the current one-session proof; replacement requires measured
  pressure and a fresh gate
- **Credible alternatives:** Asupersync; Tokio; smol; a revised owned synchronous/event-loop
  design
- **Why:** The current shape keeps PTY reads, terminal authority, client writes, signals,
  and shutdown ordering explicit without an async-runtime dependency. Before changing it,
  compare cancellation and quiescence semantics, deterministic testing, ecosystem fit, owned
  LOC, transitive and build cost, macOS feasibility, and the exact contract failure or scale
  pressure that justifies migration. Asupersync is a candidate for region ownership, bounded
  cancellation, and deterministic replay; it is not preselected, and reference use does not
  bypass license or dependency review.
- **Evidence:** Accepted proof in `orb-bi4.3`; deferred replacement gate in `orb-m73`,
  triggered only by measured contract pressure

### Repository governance metadata

- **Selected shape:** Private `orbit-governance` Rust workspace tool with `serde` 1.0.229
  and `serde_json` 1.0.151
- **Status:** Selected for repository checks; not a product dependency
- **Credible alternatives:** Python 3 standard library; an owned Rust JSON parser;
  `json` 0.12.4
- **Why:** Typed Beads input keeps Orbit's contract cross-links and crate-evidence checks in
  the Rust-owned stack. The tool does not compile or alter the Orbit runtime, and its two
  direct crates are already in the workspace graph at these versions. Fixed grammars use
  owned scanners, so regex, Markdown, CLI, error, hashing, and test-framework crates are
  unnecessary.
- **Evidence:** Crate and reference gates in `orb-9o6`; consolidated-tree record in
  `eon-monorepo-2oko.9`

### Structured presentation extraction

- **Selected shape:** Existing `libghostty-vt` 0.2.2 `RenderState`, row/cell iterators, and
  terminal grid references
- **Status:** Selected for the headless proof
- **Credible alternatives:** A small approved binding extension; libghostty fork;
  replacement by `rio-vt`; secondary oracle using `vt100` or `termwiz`
- **Why:** The public safe API supplies coherent render snapshots, styled graphemes,
  resolved background-only cells, palette and cursor data, while grid references supply
  hyperlink URIs. The same release exposes global and per-row dirty state, so a later patch
  experiment can reuse the selected dependency. Orbit selects no diff or serialization
  crate. Dirty tracking does not define patch composition, history, effects, or wire
  compatibility. A bounded encoder and output queue need no serialization, buffer, async,
  binding, or second-engine crate. The presentation implementation and co-located canonical
  proof occupy 650 lines in `src/presentation.rs`; total owned Rust is 2,322 lines versus
  1,554 before the proof. The ten-package normal graph is unchanged, and the boundary stays
  platform-neutral above the accepted PTY seam. Ghostling
  `f9034e43a50a2f3a8101e35497f486090c1ddd6e` and libghostty-rs
  `72ac98f292879bf9f788fcbb11238c562a1eebe6` confirm render ownership; justerm
  `0e468a7fde4e75d307983258d09dd8dfbe26f81a` informs complete-frame ownership without
  becoming a dependency.
- **Evidence:** Accepted `orb-bi4.3` proof `e4fde443e625180d7332eff4dff366f64bee30a8`

### Canonical Orbit consumer codec

- **Selected shape:** Private `orbit-protocol` 0.1.0 workspace library with `std` only and
  no external dependency
- **Status:** Selected and proved for ORBS v10; ORBS v9 remains the immutable prior routing
  proof, and Venus, Eon, and Eonova consume exact v10 compositions
- **Credible alternatives:** Root library target with terminal features; a second Venus
  decoder; a second handwritten session schema; bincode 2.0.1; postcard 1.1.3; prost 0.14.4;
  rkyv 0.8.17; capnp 0.27.0; a separately published protocol repository
- **Why:** One package owns ORBF frame values and reduction plus exact-version ORBS
  attachment, metadata observation roles and bounded title/CWD values, frames, lifecycle,
  semantic input, authoritative terminal-versus-host left-pointer routing, selection and
  destination-tagged copy, terminal clipboard writes, bounded multi-row previews, typed
  vertical-wheel outcomes, bounded signed whole-row viewport commits, and private management
  identities, messages, and records. Both codecs use explicit little-endian bytes, strict
  pre-allocation and mapper-domain bounds, and no generated schema; Orbit alone maps
  accepted values into authoritative libghostty state and PTY resize.
  `cargo tree --locked -p orbit-protocol` contains only the package, so a Venus build does
  not compile `libc`,
  libghostty, PTY, governance, native, build-script, or Nix surfaces. The platform-neutral
  package is credible for Linux, macOS, and a future separately authorized Wasm consumer.
  Checks cover rich frame fidelity and ordering, each message family and mouse button,
  exact-version attachment and metadata observation, revision-bound bounded multi-row
  previews, typed atomic vertical-wheel outcomes, one-request signed viewport batches with
  exact requested/applied rows and the next preview, terminal-forbidden key text, modifier
  and mouse canonicalization, routed bounded left-pointer sequences, Shift host bypass,
  libghostty-owned cell, word, and logical-line gestures, bounded Unicode copy destinations
  and terminal clipboard writes, fail-closed terminal and transport pressure, arbitrary
  paste bytes, malformed typed values, header-only role and message-length rejection,
  bounded management values, messages, and records, truncation, real-PTY input, detach, and
  reattach. General serializers either change accepted bytes or retain custom adapters;
  schema systems add generated/build and compatibility surfaces.
- **Evidence:** `orb-bi4.9`, `orb-bi4.10`, `orb-j0m.3`, `orb-ll1.1`,
  `orb-orbit-typed-wheel-outcomes-5nq`, `orb-orbit-signed-scroll-batch-x12`,
  `orb-orbit-multi-row-scroll-preview-ca7`, `orb-orbit-native-selection-gestures-qf3`,
  `orb-route-left-pointer-afz`, and `orb-implement-orbit-session-management-owner-91e`;
  accepted ORBS v2 proof `9d6d2bb37f20ab4ad9e186c7bc715eabef43e757`, v3
  `3ee7c80005f3d2bbe81e539799327803716f6174`, v4 `9c0617a97612cdd045ed67b5cd7c87244eb888e6`,
  v5 `69c402737799f03e615473956954a043647a4713`, v6
  `780f5d746175b4a9b71df57c51ed4bfcc4c4c375`, v7 `baf8aa28dcaa50484cd221aa7730defedc2356bb`,
  v8 `d9b22eb294f8f42b4f49324fd5467eab239c2917`, v9
  `aed0bcb7e9ad08c8e3e086c7dad0a0eb3ef16672`, and v10
  `59975e9176f5caf8b78dc3273e88d9ecbb75dc3f`; hardened v10 Venus consumer
  `bdd8b3628452b1dba8c4d4b2ad9bc2e802659d29` and Eon composition
  `71e8f5a8cac938ed7f065890c83d9376551f6014`; v10 Eonova composition
  `f4717efe7c68f6061209a225c2a551031da82060`

### Native Venus window, font, and renderer

- **Selected shape:** Eon Desktop pins `winit` 0.30.13, `wgpu` 30.0.0, `glyphon` 0.12.0 with
  `cosmic-text` 0.19.0, `AccessKit` 0.24.1, `accesskit_winit` 0.33.2, and `pollster` 1.0.1
- **Status:** Selected and proved in Eon Desktop; none are Eon Sessions dependencies
- **Credible alternatives:** An owned `cosmic-text` atlas; `softbuffer` plus `tiny-skia`;
  Vello plus Parley; Sugarloaf after a proved glyphon failure; no unnecessary `librio`,
  Ratatui, Qwertty, or full UI framework dependency
- **Why:** Venus owns native lifecycle, input, GPU presentation, text shaping, and
  accessibility while Orbit remains the sole terminal authority. The selected stack passed
  the native consumer proof and the corrected sustained-output envelope at Venus
  `8929c9f9d151641a343813ddeb6005cb9c771286` without changing ORBS v1 or adding an Orbit
  dependency. VEN-C16 proves Linux native Wayland at Venus
  `e033efadf023492ae02eb1e9036de98ad93d2f98`; X11, Xwayland, macOS, and other native
  platforms are unsupported. Broader Wayland compositor coverage remains unproved.
- **Evidence:** Venus `ven-upt.1`, `ven-upt.2`, `ven-39u`, and
  `ven-venus-linux-wayland-only-ndj`; Orbit acceptance in `orb-bi4.4` and `orb-bi4.5`

### Future Eon Web decoder bridge

- **Selected shape:** No selection; web scope remains unauthorized
- **Status:** Deferred until a separate web-client and transport decision
- **Credible alternatives:** Canonical Rust decoder compiled with `wasm-bindgen`; a
  browser-native decoder; a shared Wasm renderer; no web client
- **Why:** The preferred first comparison shares Orbit protocol interpretation while
  TypeScript owns browser APIs. Compiling the decoder to WebAssembly neither approves a full
  shared renderer nor creates a plugin system.
- **Evidence:** Post-graduation decision in `orb-bi4.7`; [`STACK.md`](STACK.md)

### Future cross-language extension host

- **Selected shape:** No embedded runtime or general plugin framework
- **Status:** Deferred until repeated user-approved extension cases establish a stable
  capability contract
- **Credible alternatives:** Versioned out-of-process standard-input/output messages; direct
  Wasmtime Components; Extism; another credible host; no general extension API
- **Why:** Specify capabilities and lifecycle in WIT before selecting a host. Orbit's
  authoritative owner loop remains plugin-free, and all sensitive capabilities are denied
  unless explicitly granted.
- **Evidence:** Post-graduation decision in `orb-bi4.7`; [`STACK.md`](STACK.md)

## Recording rules

- Record direct and architecture-shaping choices, not every transitive package.
- Include the no-new-crate or existing-dependency shape whenever it is credible.
- Exact releases or commits, accepted and rejected alternatives, dependency
  footprint, owned LOC effect, build implications, and evidence Bead belong in
  each completed decision.
- Record whether each candidate preserves a credible macOS implementation path
  and the platform seam it would require. This does not make portability more
  important than contract fit, ownership, LOC, or dependency cost.
- `Selected for the experiment` is not a permanent endorsement. A later Bead
  may preserve, replace, or remove the choice through a fresh crate gate and
  explicit user approval.
- Candidate code does not become an accepted crate decision merely because it
  builds or passes tests.
