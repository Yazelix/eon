# Eon stack technology boundaries

This document records the current cross-stack technology posture for Eon, Eon
Sessions, and Eon Desktop. Orbit and Venus remain the underlying session and
client subsystems. This document guides later decision beads; it does not
authorize a feature, dependency, repository, plugin system, web client, mobile
client, or Apple client. The initial Orbit experiment retains the narrower
boundaries in `AGENTS.md` and `docs/CONTRACTS.md`.

## Default ownership

The stack has a Rust center with platform-specific languages only at boundaries
where they have a concrete native advantage.

| Surface | Default owner | Boundary |
| --- | --- | --- |
| Eon Sessions / Orbit runtime and terminal authority | Rust | No second application language or in-process plugin runtime |
| Canonical Orbit protocol codec | Rust | May compile to WebAssembly for a browser consumer |
| Eon Desktop / Venus shared state and native client | Rust | Renderer dependencies require the Venus crate gate |
| Eon Desktop / Venus GPU shaders | WGSL when required | Shader code only, not application policy |
| Eon composition and packaging | Nix | Reuse child-repository outputs instead of local wrappers |
| Eon executable logic | Rust when Nix alone is insufficient | Add only for deterministic behavior that earns a typed, checked owner |
| Future Eon Web shell | TypeScript | Browser lifecycle, transport, DOM, accessibility, and PWA integration |
| Future Apple-native Eon Desktop shell | Swift and SwiftUI only if justified | Apple lifecycle, input, menus, and accessibility around a narrow Rust boundary |
| Yazi extensions | Lua, owned by Yazi | Eon may pin or package them without making Lua an Eon-wide API |
| Future cross-language extensions | WebAssembly Components described by WIT | Only after concrete extension cases establish a stable capability contract |

Upstream implementation languages stay encapsulated. In particular,
libghostty's use of Zig does not make Zig a directly owned Orbit language.

## Runtime ownership

Eon owns product composition: manifests, policy, component selection,
launching, updates, distribution, and the mapping among repositories, agents,
services, and terminal sessions. Eon Sessions contains Orbit, which owns
generic terminal-session lifetime, PTYs, authoritative libghostty state,
bounded terminal history, and the versioned client protocol. Eon Desktop
contains Venus, which owns native rendering, input collection, and ephemeral
view state over Orbit-authored presentation.

Raw PTY bytes and terminal-generated replies stay inside Orbit. Venus
materializes ORBF complete frames and does not run a terminal parser. A later
user-approved patch protocol would update the same presentation state. Eon
does not interpret terminal presentation. Future multiple-session work must
keep generic Orbit session identity separate from Eon's product workspace
policy.

Orbit owns the wire schema and its state-transition rules. The
`orbit-protocol` package owns canonical ORBF values, bounded
decoding, complete-frame reduction, and exact-version ORBS
messages for one input-capable attachment, one bounded title/CWD observer,
frames, lifecycle, semantic input, selection and copy, terminal clipboard
writes, bounded multi-row previews, typed vertical-wheel outcomes, and bounded
signed viewport commits. The [contract index](CONTRACTS.md) records accepted
producer and consumer revisions. Venus must record each consumed contract's
proof revision and pin the exact package revision instead of mirroring the schema.

## Two distinct WebAssembly roles

### Browser code sharing

A future Eon Web client may compile the canonical Rust frame decoder and
client-state transitions to WebAssembly. TypeScript should own browser APIs and
transport.
The first decision should share protocol interpretation, not assume that the
entire native renderer must also run through WebAssembly.

This is a client implementation technique. It does not create a plugin system,
expand the Orbit contract, or authorize remote attachment.

### Sandboxed extensions

If Eon or Venus later needs a language-neutral plugin boundary, define the
accepted host capabilities in WIT and evaluate WebAssembly Components. Select a
host such as direct Wasmtime or a higher-level framework such as Extism only
after the interface exists and through the owning repository's crate gate.

This is a product extension mechanism. It is independent from compiling the
web decoder to WebAssembly.

## Extension sequence

Do not build a general plugin framework in anticipation of plugins.

1. Implement a user-approved feature in its owning repository.
2. Wait until at least two separate useful extension cases expose a repeated,
   stable capability boundary.
3. For trusted integrations, first compare a versioned out-of-process
   request/response contract over standard input and output with no framework.
4. If isolation, distribution, or multiple implementation languages justify a
   runtime, specify capabilities and lifecycle in WIT before selecting a host.
5. Compare direct Wasmtime, Extism, another credible host, and no embedded
   runtime through the crate gate.
6. Keep filesystem, network, PTY, process, secrets, clipboard, and persistence
   access denied unless an explicit capability and user decision grants it.

Native Rust dynamic libraries are not the default extension shape because they
lack a stable Rust ABI and weaken portability and isolation. Embedded Lua,
Python, Rhai, Starlark, JavaScript, or another scripting runtime requires a new
decision; an existing child product's plugin language does not establish an
Eon-wide precedent. MCP may expose Eon actions to agents or tools later,
but it is not the product UI or runtime plugin ABI.

## Extension placement

- Orbit remains the smallest authoritative PTY and terminal-state owner. It
  does not host arbitrary plugins in its owner loop. A future read-only observer
  or exporter would require a separate contract and failure-isolation decision.
- Eon may expose orchestration, workspace metadata, launch-policy, or action
  capabilities. It must not absorb behavior already owned by a child repository.
- Venus may expose commands, status items, sidebars, inspectors, or panels. UI
  contributions should use host-rendered declarative data and events rather
  than raw GPU, window, or terminal authority.
- Yazi Lua plugins remain Yazi plugins. Eon may select, pin, and configure
  them through Yazi's supported surface.

Every accepted extension contract names one owner, its capabilities, lifecycle,
failure behavior, versioning, compatibility policy, and cheapest proof. A
cross-repository extension cannot hide a missing Orbit, Venus, Eon, or
child-repository contract.

## Venus framework gate

The first native Venus slice remains Rust. Its crate gate should compare the
smallest credible window, GPU, font, shaping, and text-rendering ownership
shapes. `winit`, `wgpu`, `glyphon`, and `cosmic-text` form one plausible
candidate stack, not a preselected dependency set. Compare them with the
relevant Ghostling, libghostty-rs, and renderer evidence at exact revisions.

The gate must measure project-owned LOC, dependency and build cost, text and
grapheme correctness, input methods, accessibility, Linux behavior, macOS
feasibility, and whether a browser path remains credible. WGSL is acceptable
only where the selected GPU stack requires shaders. Ratatui may support a
lossy diagnostic or administration client, but it is not the native Venus
graphical framework and cannot prove rich-presentation fidelity.

## Deferred platform decisions

- TypeScript and browser WebAssembly become implementation scope only after an
  Eon Web contract and transport decision are authorized.
- Swift and SwiftUI become implementation scope only when measured Apple
  integration needs justify a separate shell over the Rust core.
- Kotlin, Compose Multiplatform, Flutter, Electron, Tauri, Qt, and a shared
  Wasm renderer remain rejected defaults, not permanent bans. Reconsider one
  only against a concrete platform contract and measured advantage.
- Remote attachment, web presentation, mobile presentation, and remote
  workspace orchestration remain separate decisions.
