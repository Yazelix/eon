# 2. Window and renderer: from state to pixels

Verified source: `073f03cd2b116ba885e2cffde9fef56793395740`.
<!-- owner-only:start -->
**Owner only.** Budget: idea 4 minutes, commands 3, code 6, critic 4, explain it back 3.
<!-- owner-only:end -->

## The idea

A **renderer** draws a description of a scene onto a display. In Eon, Venus
consumes Orbit-authored terminal frames and turns them into native draw inputs.
Orbit retains terminal authority: Venus does not become another parser of the
program's terminal output ([venus/docs/CONTRACTS.md:230](../../venus/docs/CONTRACTS.md),
[orbit/docs/CONTRACTS.md:85](../../orbit/docs/CONTRACTS.md)).

At this revision Venus uses `winit` for window integration, `wgpu` for graphics
and `glyphon` for text drawing. Its Linux features select Wayland, the display
protocol, and Vulkan, a graphics interface; its macOS graphics feature selects
Apple's Metal interface. Those are implementation choices in
the source, not proof that all graphics drivers or desktop environments work
([venus/Cargo.toml:7](../../venus/Cargo.toml),
[venus/Cargo.toml:25](../../venus/Cargo.toml),
[venus/src/render.rs:10](../../venus/src/render.rs)).

The code distinguishes rebuilding content from updating moving cursors. A
**viewport** is the area the renderer draws into; a **glyph** is a drawn letter
or symbol. During frame preparation, Venus can reuse content that did not
change while constructing current cursor geometry
([venus/src/render.rs:2313](../../venus/src/render.rs),
[venus/src/render.rs:1870](../../venus/src/render.rs)).

<!-- owner-only:start -->
Owner terminology: “the Rune move” names a renderer-migration topic in the
class brief. This checkout does not establish its outcome. The renderer
you inspect here imports wgpu/glyphon and declares winit/wgpu dependencies.
This guide claims no shipped Rune renderer or migration outcome from those
files. Future work needs its own source and proof
([venus/src/render.rs:10](../../venus/src/render.rs),
[venus/Cargo.toml:7](../../venus/Cargo.toml),
[docs/CONTRACTS.md:22](../CONTRACTS.md)).
<!-- owner-only:end -->

## Run it

From the repository root, in Nushell:

```nu
^nl -ba venus/src/render.rs | ^sed -n '1838,1890p'
^nl -ba venus/Cargo.toml | ^sed -n '7,37p'
```

## What to look at

Find `rebuild_if_needed`, followed by `build_dynamic_cursors` and
`upload_dynamic_vertices`. Notice that `viewport.update` sits inside
`if content_changed`. In the manifest output, distinguish common dependencies
from the Linux and macOS feature sections
([venus/src/render.rs:1854](../../venus/src/render.rs),
[venus/Cargo.toml:25](../../venus/Cargo.toml)).

<!-- under-the-hood:start -->
<details>
<summary>Under the hood: code walkthrough (optional)</summary>

## Read the code

Walk [venus/src/render.rs:1838-1890](../../venus/src/render.rs):

- Lines 1838–1851 name the inputs: terminal scene, scrolling preview,
  workspace, columns, dialog, focus, status and input-method text (`preedit`).
  `Option` means an input may be absent. The return type allows a render error.
- Lines 1852–1853 start measurement of the content rebuild. These timestamps
  help inspect the path; they do not establish a performance result here.
- Lines 1854–1866 pass the inputs to `rebuild_if_needed` and remember its
  change decision.
- Lines 1867–1869 finish one timing measurement and start the next.
- Lines 1870–1878 construct dynamic cursor geometry and upload its vertices,
  the points the graphics hardware uses for drawing.
- Lines 1879–1890 update the viewport dimensions when content changed. The
  excerpt ends before text preparation; follow from line 1891 if you want to
  see glyphon prepare the text and retry after trimming its glyph atlas,
  a store of drawn glyph images
  ([venus/src/render.rs:1891](../../venus/src/render.rs)).

</details>
<!-- under-the-hood:end -->

## The hardest question

> “Why own a native renderer, and what does replacing it cost?”

Eon's chosen boundary lets a native client render structured terminal state
without taking terminal ownership. It also leaves the project responsible for
text quality, graphical interaction and platform behavior. VEN-C1 labels native
presentation partially proved and broader native quality unproved. A future
renderer change would need evidence for the affected output and installed
interaction, not just a successful compile
([venus/docs/CONTRACTS.md:230](../../venus/docs/CONTRACTS.md),
[docs/AGENT-RULES.md:88](../AGENT-RULES.md),
[docs/AGENT-RULES.md:91](../AGENT-RULES.md)).

That is a defensible tradeoff, not a measured speed advantage over another
terminal. This excerpt explains the changed-content path; it supplies no
benchmark.

<!-- owner-only:start -->
## Explain it back (owner only)

Point out where content reuse and cursor movement diverge. Explain why swapping
the renderer need not transfer terminal-parser ownership from Orbit.
<!-- owner-only:end -->

## Going deeper

- [Native presentation contract, venus/docs/CONTRACTS.md:230](../../venus/docs/CONTRACTS.md).
- [Terminal-state authority, orbit/docs/CONTRACTS.md:85](../../orbit/docs/CONTRACTS.md).
- Next: [the supervisor](03-runtime-supervisor.md).
