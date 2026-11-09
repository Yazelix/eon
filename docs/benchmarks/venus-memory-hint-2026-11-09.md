# Venus MemoryUsage allocation experiment — 2026-11-09

**Outcome: MemoryUsage for non-CPU adapters; Performance for CPU adapters.**
Bead `eon-ypy0a.2` required a real GPU-memory reduction without a meaningful
frame or input regression. Requesting wgpu 30's `MemoryHints::MemoryUsage`
saved **180.3515625 MiB** of allocated and resident Intel DRM memory in each
of three fresh-client observations, passing all predeclared Intel timing
gates. The software input proxy failed its gate; pixel bounds did not
establish a matching visible slowdown. CPU adapters keep their existing
allocation hints.

The initial conservative outcome was to revert the hint entirely. At
05:26:26Z the overseer, under Lucca's delegated approval, superseded that
outcome with the adapter selection rule: `device_type != Cpu` requests
`MemoryUsage`, while `Cpu` retains the measured `MemoryHints::Performance`
baseline. Existing Intel B evidence proves the changed hardware policy;
existing lavapipe A evidence proves the retained CPU policy. No new timed
comparison is used to overturn the software result. The final conditional
build passed native correctness smokes on both actual adapters and the
required suites/package checks.

This is a diagnostic A/B experiment, separate from the published
[ten-launch native-client cohort](venus-client-memory-2026-11-08.md).
No setting, dependency, custom allocator knobs, renderer/surface path, contract
or wire changes. The fixed adapter allocation policy is the approved memory
budget choice. CPU saving budget: zero. Metal and other hardware remain
unmeasured; the non-CPU rule is the approved policy, not universal timing proof.

## Revisions and isolation

A is detached baseline `f7477be6d698a80787353c58afb43eae4da2be0a`.
B's Git tree is `cf82643a81c80dfb0da0aedc0b5f3e15734d1dc1`, the unconditional
experimental hint in `Renderer::new`:

```rust
.request_device(&DeviceDescriptor {
    memory_hints: wgpu::MemoryHints::MemoryUsage,
    ..Default::default()
})
```

The final implementation selects that field through a private helper. Its
unit test covers `Other`, `IntegratedGpu`, `DiscreteGpu` and `VirtualGpu`
selecting `MemoryUsage`, and `Cpu` explicitly selecting `Performance`.
The device request retains every other descriptor default.

Immutable Nix package outputs:

| Arm | Package | Actual Eon executable |
| --- | --- | --- |
| A | `/nix/store/vwaxmqwb6rvlimx70b64v0wl70jiwxdj-eon-0.1.22` | `/nix/store/6z517rs0910588agyii85ggcvi92djc8-eon-0.1.22/bin/eon` |
| B | `/nix/store/k7a1wfjxgx17br3q7k1g52g9jj4n5mml-eon-0.1.22` | `/nix/store/k9mc4m4bf5z28qii17qyrd888vyznp5l-eon-0.1.22/bin/eon` |

Both use the identical baseline-built benchmark:
`/nix/store/ckyxmmy9qrg7qrspk3wqdpx8nbp6p80y-eon-bench-0.1.0`;
wrapped binary SHA-256
`b025974df4105420680c63c6a0ad7b0adc3a771d91f72fa43b3e1e308215ff5c`.
The result JSON build revision identifies this harness, while its environment
and executable hashes identify the measured subject. Release builds, no
affinity pin, powersave governor, Pop!_OS 24.04 LTS, Linux
7.2.6-76070206-generic, Intel i5-13450HX, 16 available CPUs, Mesa 26.1.2.

All observations use the existing eon-bench `Headless` launcher, private Sway
1.12 at 1280×800, HOME, Eon roots, D-Bus and PID/network namespaces. Intel
uses GLES Sway and `/dev/dri/renderD128` (PCI `0000:00:02.0`, Intel Graphics
RPL-S, i915); actual Venus Vulkan adapter must be hardware. The full pinned
ICD discovery environment is unchanged between arms. Lavapipe uses pixman
Sway and the pinned Mesa lvp ICD; actual Venus adapter is llvmpipe Vulkan,
LLVM 21.1.8. No desktop focus, live Eon window/supervisor, profile or host
display was changed. Only owned isolated processes are stopped.

## Memory

Each memory run uses the landed Venus `client-50-filled` row, one fresh
launch, no warmup, 3 s settling, 50 terminals at 80×24 with 10,000 numbered
72-column lines each and one visible tab. Exact corpus SHA-256:
`1febbe9a5482beaee070f12e890dbf5f1ff0dd6cd4a30388f8c4304d5a432e7e`.
All twelve runs passed filled-screen, attachment, dimensions and actual
native-client/adapter oracles. Both arms also passed `eon-bench headless`
on each adapter. Memory runs were A1–A3 then B1–B3, not interleaved; their
natural shared-load values remain in the raw results.

```text
eon-bench memory --eon <arm>/bin/eon --renderer <node-or-lavapipe>
  --subject eon --samples 1 --warmups 0 --settle-ms 3000
  --max-load 64 --load-wait 0 --deadline 300 --out <private-run>
```

The external collector follows only descendants of the exact launcher PID
and captures Venus PID/start-time, executable, namespace, load, `smaps`,
`smaps_rollup` and raw DRM fdinfo every 0.5 s. Final-client last three complete
captures reproduce the stable Intel values below in every launch. DRM
descriptors are deduplicated by `(drm-pdev, drm-client-id)` before summing:
two clients, one with no allocation counters. Duplicate-fd reducer self-check
passed. These driver counters can include shared buffers and overlap process
mappings; they must not be added to PSS or called uniquely owned RAM.

| MiB, each of three Intel launches | A | B | A − B |
| --- | ---: | ---: | ---: |
| DRM allocated | 230.500000 | 50.148438 | 180.351563 |
| DRM resident | 228.500000 | 48.148438 | 180.351563 |
| DRM shared | 7.921875 | 7.921875 | 0 |
| DRM purgeable | 0.515625 | 0.164063 | 0.351563 |

Lavapipe has no corresponding DRM hardware counters. Its named
`memfd:allocation fd` mappings total 64.01953125 MiB virtual in every A
memory launch and 4.01953125 MiB in every B launch. A resident mapping totals
are 1.4609375–1.640625 MiB; B is 1.45703125 MiB. The 60 MiB virtual shrink
is not a 60 MiB resident saving.

The suite's native-client estimator is after(server + Venus) minus
before(server), with workloads excluded. Medians across three independent
launches, MiB:

| Adapter / arm | PSS delta | RSS delta | Pss_Anon delta |
| --- | ---: | ---: | ---: |
| Intel A | 66.051 | 86.395 | 30.891 |
| Intel B | 51.820 | 86.430 | 30.676 |
| lavapipe A | 102.559 | 112.711 | 27.262 |
| lavapipe B | 86.896 | 113.070 | 27.648 |

PSS varies with unrelated sharing; RSS and anonymous PSS do not demonstrate
an attributable CPU saving. [Standalone Venus process counters](venus-memory-hint-2026-11-09/memory.csv)
are a separate snapshot diagnostic, not the suite delta estimator. Its
software DRM zeros mean unavailable, not a measured zero-memory GPU.

The [churn memory CSV](venus-memory-hint-2026-11-09/churn-memory.csv) retains
every phase's observed ranges and last sample. Intel resize peaks were
235.613 MiB allocated / 233.613 resident in A, 55.258 / 53.258 in B. Text and
image churn did not undo the Intel saving. Software mappings after image
deletion retained about 9.215 MiB resident in A and 11.215 MiB in B; the
smaller virtual pool does not imply smaller resident image-churn memory.

## Timing and visible behavior

Frozen disposable driver sources accompany each run. They reuse the exact
existing Rust Headless launcher; no product or benchmark source was edited.
Phases: idle; 20 resize cycles concurrent with rich Latin/CJK/emoji/combining
text and Kitty images; 60 native inputs during 512² RGB/RGBA replacement
churn at 150 ms intervals; one 1024² RGBA image; image deletion. Each run
verifies 96 exact echoed input counters and their Orbit revisions. Keys are
not injected during transient resize geometry.

CPU frame time is trace `render_begin` to successful `present_revision`,
not GPU execution time. The input proxy matches native key ingress to a
presented revision at least as new as the exact echoed counter's queried
revision. Unrelated newer content can make this proxy conservatively late.
The separate visible-response upper bound ends when a private grim capture
confirms the opposite black/white response-band pixel. CLI/capture overhead
is included; it is not photon latency. Both distributions are retained.

Every final run has zero trace-producer loss, complete sequence coverage,
96 pixel-confirmed responses, rich-text screen checks and image-presence/
deletion checks. Green image pixels are 72,400 on Intel and 72,392 on
software, falling to 2,846 after deletion. All 14 runs pass; no missing image
can count as a memory win.

Three matched rounds per adapter use orders A→B, B→A, A→B (pair IDs 2–4).
The pre-B plan defined meaningful regression as **>10% and >1 ms** in a
phase's median or p95, consistently in at least two of three matched pairs
with the pooled distribution agreeing. Contradictory results permit one
additional matched pair, not repeated retries for a favorable outcome.
All maxima remain in the [original comparison JSON](venus-memory-hint-2026-11-09/timing-comparison.json).
Idle visible-response maxima were 93.921/93.748 ms Intel A/B and
81.405/87.218 ms software A/B, within the 250 ms idle response budget.

The original pooled results follow. Each cell is median / p95 in ms;
frame counts and all individual inputs remain in the archive.

| Intel phase | CPU frame A → B | Input proxy A → B | Visible upper A → B |
| --- | ---: | ---: | ---: |
| idle | 1.045/7.183 → 1.020/6.370 | 21.046/25.170 → 19.820/24.196 | 64.739/92.139 → 62.397/91.281 |
| resize | 1.126/7.389 → 1.104/7.400 | not measured | not measured |
| churn | 1.074/6.597 → 1.052/6.432 | 22.174/55.652 → 20.188/40.751 | 70.049/88.485 → 68.936/85.505 |
| image | 0.918/4.656 → 0.986/4.696 | 18.962/24.217 → 18.872/23.721 | 63.922/83.155 → 61.556/82.842 |
| delete | 0.994/6.445 → 0.866/3.506 | 19.867/26.009 → 18.671/22.988 | 66.115/84.492 → 61.298/78.088 |

| lavapipe phase | CPU frame A → B | Input proxy A → B | Visible upper A → B |
| --- | ---: | ---: | ---: |
| idle | 4.749/10.295 → 5.320/11.245 | 22.239/28.357 → 24.406/29.320 | 65.803/78.673 → 61.395/77.137 |
| resize | 5.669/9.934 → 5.899/10.030 | not measured | not measured |
| churn | 5.503/10.283 → 5.703/10.796 | 21.682/50.882 → 24.295/55.421 | 64.029/87.420 → 66.064/87.933 |
| image | 5.360/10.690 → 5.518/10.809 | 22.689/29.183 → 26.120/30.017 | 57.949/76.720 → 58.276/80.470 |
| delete | 5.291/10.478 → 5.218/10.120 | 23.378/29.349 → 22.875/28.880 | 62.045/87.028 → 61.963/86.144 |


The only original gate flag was software static-image input proxy median:
22.689→26.120 ms (+3.431 ms, +15.12%). Matched medians were
17.877→22.677, 27.692→26.120, and 22.689→27.729 ms. Pixel bounds and frame
times did not consistently corroborate it. The one additional software
pair ran B→A: static-image input proxy 5.965→25.384 ms (A→B values), frame
CPU median/p95 2.113/4.231→4.848/11.217 ms, visible upper median/p95
59.170/71.520→61.693/75.602 ms. Across all four software pairs the static
input medians are 20.506→25.934 ms. The
[extra pair and four-pair summaries](venus-memory-hint-2026-11-09/timing-extra-comparison.json)
remain separate from the original comparison.

The extra pair's A frame/input timings were much lower across several
phases. Frequency, thermals and scheduling were not controlled enough to
attribute that difference to the hint. These data cannot support accepting
the unconditional hint on CPU adapters. The delegated ruling therefore keeps
the baseline policy for CPU devices and accepts the Intel-proved hint for
non-CPU devices. No custom allocator tuning was tried.

Timed cases ran through heavy.sh. The first batch started at 04:56Z under
ordinary scheduling, before the overseer's 04:58:46Z quiet grant; the initial
cases therefore had no reserved quiet window. The remainder and extra pair
ran during it. Capture load ranged 1.31–4.94, no affinity/frequency control.
The extra pair used the authorized `HEAVY_QUIET_EXEMPT=1`; QUIET DONE was
reported 05:12:11Z. Native timing is local evidence, not a claim about Metal,
other hardware, or universal latency.

## Attempts, proof and artifacts

Baseline pilot failures are retained in the archived numbered bead records:
startup/readiness and focus calibration in the private display; trace
datagram loss; an overly fast 4 MiB-image baseline stall; and existing
response-suite startup/inconclusive attempts. None enters the paired cohort.
The software v2 pilot's visible churn maximum 1092 ms is retained as a
baseline limitation. The final paired maximum was 209.455 ms in B Intel
churn, versus A 99.683; software A/B churn maxima 151.223/92.855 ms.

The private network namespace initially had UNIX datagram queue 10 versus
host 512. The final driver sets only its private queue to 512, verifies UID
mapping and distinct namespace, and asserts host queue unchanged. The
setting is identical A/B and transports trace records; no GPU allocator
knob changes. All source revisions, commands, cleared/private environments,
process identities, native before/after snapshots and actual adapter records
remain in the evidence. Derived CSV/JSON assertions check cohort sizes,
trace loss/coverage, exact input counts, image pixels, and nonempty software
allocation mappings.

The conditional build passed fmt and all-target Venus clippy with
`-D warnings`. Its Venus suite passed 235 library, 93 binary and 21
integration tests (349 total), with 13 expected native ignores and zero
failures. Whole-workspace `nix develop -c eon-test` passed 1,102 tests with
68 ignores and zero failures. The Nix package passed 915 tests, with zero
failures and 67 ignores.

Both `eon-bench headless` native correctness smokes passed with the
conditional package: Intel hardware Vulkan and lavapipe CPU Vulkan. Each
verified a private 1280×800 display, native presentation, managed Stop and
cleanup. These smokes are separate from the timed comparison cohorts.

The raw archives are held **outside the repository** at
`/home/lucca/.cache/eon-agents/eon-wgpu-hint-work/evidence/` until
`eon-benchmarks-xl2bp.8` settles evidence storage. CSV and JSON summaries
remain in the repository. The [SHA-256 manifest](venus-memory-hint-2026-11-09/SHA256SUMS)
retains the archive checksums as well as the summary checksums:

- `memory-evidence.tar.gz`: all 12 suite results/native evidence and
  final-client raw maps/fdinfo.
- `timing-evidence.tar.gz`: all 14 final traces, actions, phase times,
  pixels/full captures, driver sources, raw memory tails/extrema, plus
  excluded pilot records.

Each archive also contains a per-file SHA256SUMS manifest. Disposable private
home/cache contents and binaries are excluded; full raw originals remain in
the work directory. Archived scripts are experiment evidence, not supported
tools.
