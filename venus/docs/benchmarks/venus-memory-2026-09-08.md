# Venus cell-buffer memory

`ven-profile-venus-memory-u3o`, 2026-09-08. Source
`541a9cb43c155b8b97069904593dc81c73682613` is mechanically verified and
dogfooded on an isolated native Wayland display. This is a Venus allocation
correction, with no protocol, font policy, dependency or Eon pin change.

At 120×30, paired median Venus PSS savings were **4.01 MiB for a first screen**
and **4.27 MiB for loaded Unicode**. Idle memory was effectively unchanged.
These results use Sway with software Vulkan; they do not revise the earlier
COSMIC/Intel benchmark or establish installed Eon savings.

## Change and attribution

`Renderer::push_cell_text` creates one cosmic-text buffer per visible
authoritative cell/layer. `Buffer::new` initializes and shapes an empty line,
growing its line vector to four slots. The subsequent rich-text assignment
reuses that allocation for one cell. The correction uses `Buffer::new_empty`
and `lines.reserve_exact(1)` before the existing text and shaping calls.
Additional lines can still grow normally. No cache policy or helper is added.

The frozen [Eon benchmark](https://github.com/Yazelix/eon/blob/418c3889b7a79808c05e6b7ead9d27d30337f496/docs/benchmarks/eon-zlf-2026-09-08.md)
attributed 67.35 MiB idle and 78.05 MiB first-screen PSS to Venus in EonTerm.
Its retained rollups further separate those medians:

| Original COSMIC workload | Anonymous PSS | File PSS | Shared-memory PSS |
|---|---:|---:|---:|
| Empty | 30.47 | 36.85 | 0.03 |
| First screen | 41.24 | 36.80 | 0.03 |
| Loaded Unicode | 42.24 | 36.86 | 0.03 |

Values are MiB; component medians need not sum to the median total. The
first-output increase was mostly anonymous memory. Those rollups do not
identify individual original allocations or assign file PSS to specific
libraries; that part of the original baseline remains unresolved.

The isolated attribution used full `smaps` and Heaptrack. Its larger file
footprint includes software-renderer LLVM mappings. Font-file mappings in the
exploratory samples occupied less than 0.2 MiB PSS; wholesale font-cache
replacement was not justified. Heaptrack identified 4.57 MB peak allocation
in the empty-line vector path alone, plus other shaped-text allocations.
Merged allocator peaks are clues, not simultaneous retained totals.

In the final screen comparison, anonymous PSS fell from 40.10 to 36.00 MiB;
file PSS stayed near 73.1 MiB and shared-memory PSS near 9.1 MiB. The measured
change removes excess line capacity and unnecessary empty-line shaping. It
does not establish that the remaining renderer or driver memory is waste.

## Fixed comparison

Venus process only, MiB. Five fresh processes per build/workload; each row
reports median and empirical p05–p95. Paired savings match repetition indices.

| Workload | Baseline PSS | Candidate PSS | Paired PSS saved |
|---|---:|---:|---:|
| Empty | 110.37 (110.27–110.53) | 110.30 (110.20–110.48) | 0.06 (-0.11–0.27) |
| First screen | 122.34 (122.14–122.52) | 118.27 (118.16–118.46) | 4.01 (3.70–4.35) |
| Loaded Unicode | 123.63 (123.62–123.89) | 119.42 (119.24–119.56) | 4.27 (4.14–4.53) |

| Workload | Baseline USS | Candidate USS |
|---|---:|---:|
| Empty | 106.80 (106.68–106.95) | 106.73 (106.63–106.89) |
| First screen | 118.76 (118.55–118.94) | 114.68 (114.57–114.88) |
| Loaded Unicode | 120.05 (120.03–120.31) | 115.83 (115.65–115.98) |

The predeclared threshold was at least 2 MiB median PSS savings in both loaded
cases and no idle increase above 1 MiB. Both loaded differences and every
individual paired loaded saving exceeded 2 MiB. Small samples give limited tail
precision; these ranges are not confidence intervals.

- Baseline Rust/Cargo sources: accepted Venus
  `ec80e36625dec73544c0cb64becf4b135c932a63`, unchanged through planning checkout
  `4ac298156b2856d2cf4feb2af346bf81fcb23820`.
- Both binaries: `cargo build --release --locked`, rustc 1.96.0
  `ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96`, x86_64 Linux. Baseline SHA-256
  `1c01d3800bc50ec6e8e5c3eae8854f2ada3e5beedc62c383915a736b89b3acb8`;
  candidate `84f6278babcb6264607aedb1e075e638de0baae8275af3febe6a4dfe2905db03`.
- Pop!_OS 24.04, kernel `7.1.5-76070105-generic`, Intel i5-13450HX, warm active
  host. Private Sway 1.12 headless output, pixman compositor, 1920×1080 at scale
  1; Venus uses Mesa 26.1.2 lavapipe Vulkan. No active-desktop window was opened.
- Identical Nix native libraries, glibc 2.42 loader and packaged font config;
  DejaVu Sans Mono, 16 logical px, 18 px rows, opaque background, no blur.
  Orbit `91999d79546422b49bdbc124166a65859d0bd872` and the installed EonTerm
  supervisor were held fixed. Only the Venus executable changed.
- Thirty measurements: baseline/candidate × empty/screen/Unicode × five
  repetitions, shuffled within each repetition with seed `20260908`. Two
  additional visual cases are excluded from the memory distributions.
- Fresh private configuration/cache/state/runtime roots per launch. Identical
  payload bytes from `eon-zlf`: 30 rows for screen, 10,030 rows for Unicode,
  without a trailing newline. DSR parsing completion, actual 120×30 geometry,
  then three seconds settling precede the sample. Retained history length is
  not measured. All sampled `SwapPss` values were zero.
- One measured Venus process at a time; separate baseline/candidate executable
  files, the same surrounding host and compositor. Shared mapping ownership
  and host load were not globally frozen; USS accompanies PSS for that reason.
- PSS and USS follow [Linux proc accounting](https://docs.kernel.org/filesystems/proc.html).
  USS sums private clean, dirty and hugetlb pages. Quantiles interpolate at
  `(n - 1) × p`. This measures neither dedicated GPU allocations nor throughput,
  latency, long-term growth, multi-window scaling or other compositors.

## Correctness and rejected experiments

All 32 captured PNGs matched their workload's baseline byte for byte. The
extra corpus includes regular/bold Nerd Font symbols, italic text, combining
marks, Greek/Cyrillic, Braille, box/block cells and foreground/background styles.
This proves preservation against the captured baseline, not universal glyph
quality. Existing fractional-scale and broader compositor gaps remain open.

Formatting, locked check, all 120 ordinary tests, and strict all-target Clippy
passed. The existing isolated native
`live_output_keeps_application_input_admitted_before_repaint` regression passed
with the candidate. No literal-capacity mirror test was added; the retained
memory comparison exercises the allocation's consumer.

Releasing shaped buffers after glyphon preparation preserved the same images
but failed to lower PSS (first-screen pilot 122.21 versus 122.72 MiB). That
candidate was rejected; the accepted correction retains the existing lifetime.
No allocator trimming, cache flushing or reduced fallback coverage was added.

Exploratory nested runtime paths failed startup; shortening the private paths
resolved it. Wrapper-owned Sway processes left stale private sockets, so the
final runs used the directly owned unwrapped compositor and verified cleanup.
A GLES renderer attempt failed initialization and did not establish a hardware
baseline. None of those attempts enters the fixed comparison.

Heaptrack source `9db5d53df554959478575e080648f6854d362faf` (reporting 1.6.80)
was a disposable development tool from pinned nixpkgs
`567a49d1913ce81ac6e9582e3553dd90a955875f`. Text analysis and the scalar timeline
worked; detailed Massif export crashed and its incomplete output was discarded.
Profiler-instrumented PSS is excluded from the comparison.

## Evidence and reproduction

[Process observations](venus-memory-2026-09-08.csv) retain all 128 process
rollups from the 32 final launches, including the unchanged supervisor, Orbit
and inert workload child. Their rows are not included in Venus totals above.
CSV SHA-256: `9754d6ccc38c77cfef5fd418e58f3222348a08206e5e9ccde457a62d3d93fc33`.

Raw captures, full Venus maps, cleanup records, exact wrappers/binaries,
payloads, profiler traces, source references, check logs and reproduction scripts
are retained with `SHA256SUMS.json` at:

```text
/home/lucca/.local/state/eon/proofs/ven-memory-541a9cb43c155b8b97069904593dc81c73682613/
```

`python3 analyze.py` verifies the retained raw rollups, CSV, 33 distributions,
PNG identities and savings thresholds without opening a window. To reproduce
the experiment, copy `measure.py`, `run.py`, `child.py`, both `venus-*` binaries
and `payloads/` to a distinct empty disposable root and run `python3 measure.py`
with the recorded Nix store artifacts available. It creates its own headless
display and refuses to overwrite an existing sample. `native.py` records the
loader/environment and exact existing native test command.

Every measured process identity and disposable sample root was gone at cleanup.
The active desktop and live user Sessions were preserved. Eon integration and
profile refresh are outside this bead; the installed alpha still uses the
earlier accepted Venus revision.
