# Venus native-client memory — 2026-11-08

MiB, median (nearest-rank p05–p95), ten fresh launches after two warmups.
Lower is better. Each client is attached to a server holding 50 terminals,
each filled with 10,000 numbered 72-column lines at 80×24; one tab is visible.
The estimator is **after(server + native client) − before(server)**, with
workload processes excluded. RSS repeats shared mappings; PSS is the primary
comparison. Anonymous PSS is shown alongside both.

| Native client | n | PSS | RSS | Pss_Anon |
|---|---:|---:|---:|---:|
| tmux (earlier reference cohort) | 10 | 0.990 (0.958–1.047) | 5.555 (5.520–5.637) | 0.756 (0.754–0.762) |
| zellij (earlier reference cohort) | 10 | 139.165 (137.356–139.904) | 146.840 (144.891–147.832) | 131.506 (129.582–132.332) |
| Eon / Venus — lavapipe software | 10 | 101.097 (85.141–101.303) | 111.367 (111.160–111.555) | 26.268 (26.113–26.402) |
| Eon / Venus — Intel hardware | 10 | 65.775 (65.485–68.718) | 85.461 (85.176–88.418) | 29.684 (29.664–32.898) |

Eon uses substantially more process PSS than tmux. Reducing Eon's memory
toward tmux is tracked in `eon-ypy0a` (`br show eon-ypy0a`); this table
retains the loss. Hardware and software allocations differ, so the two Venus
rows are separate. Memory outside process mappings, including additional GPU
or VRAM allocations, is not counted.

Venus uses an actual Eon workspace with 50 filled tabs. Before attachment,
the measured set is the supervisor and 50 Orbit host/Bend-worker pairs
(101 processes); after, the identical set plus one fresh Venus (102). All 50
workload identities remain alive and excluded. The private Sway, D-Bus,
harness, and setup presentations are excluded. Two earlier Venus clients warm
the server before the baseline; retained allocations remain in both snapshots
and can favour Eon. Tmux had no prior client. Orbit uses its runtime history byte
budget; references request 10,000 history rows. Equal output does not promise
equal retained history. The startup Files tab is closed
before the cohort, so no extra terminal engine enters the set. Eon's metadata
observers remain connected in both snapshots; Venus adds one presentation on
the visible tab. Every filled viewport and its 80×24 dimensions are verified.

The reference rows are the accepted earlier memory cohort from
`eon-benchmarks-xl2bp.5` (landing `39bc1f0`, accepted tree `a19597c3`), on the
same machine, with tmux 3.6a and Zellij 0.44.3. They were not interleaved with
these later Venus cohorts. Their sample/snapshot load range was 0.980–3.134.
Their JSON is retained, rather than presenting them as a new measurement.
Tmux and Zellij native clients render to private PTYs; no outer GUI terminal
is included. Venus includes its own window and graphics libraries. Zellij
setup used a temporary native client to establish its grid; those server
allocations may remain. The outer-terminal/multiplexer comparison is a
separate suite (`eon-benchmarks-xl2bp.17`).

The codec client remains a **server-attribution diagnostic**, not a native
client comparison. It attaches 50 sockets to the independent-engine cohort,
which has one terminal-mode supervisor and 49 additional independent engines.
It is a different cohort from the actual workspace used by Venus. Run A
contains the software native-client measurements and run B the Intel ones;
the codec diagnostic itself uses no renderer:

| Diagnostic only | n | PSS | RSS | Pss_Anon |
|---|---:|---:|---:|---:|
| Eon codec server attribution / run A | 10 | 3.125 (3.109–3.137) | 3.125 (3.125–3.137) | 3.125 (3.125–3.137) |
| Eon codec server attribution / run B | 10 | 3.129 (3.125–3.150) | 3.129 (3.125–3.133) | 3.129 (3.125–3.133) |

Environment: Pop!_OS 24.04 LTS, Linux 7.2.6-76070206-generic x86_64,
13th Gen Intel Core i5-13450HX, 16 online/allowed CPUs, powersave governor,
no affinity pin. Private Sway 1.12 headless, 1280×800, one floating 80×24
Venus client, private HOME, runtime roots and D-Bus, bubblewrap PID/network
namespaces. Software uses Sway pixman and the pinned Mesa lavapipe Vulkan ICD;
hardware uses GLES Sway and only Intel `/dev/dri/renderD128`, with pinned
Mesa 26.1.2. Software fallback fails hardware measurements. No live Eon window,
profile, supervisor, display, or desktop focus was changed. No quiet period
was reserved; natural shared load is retained beside every sample/snapshot:

- lavapipe: snapshot load 4.354–12.383; actual GPU `software=true AdapterInfo { name: "llvmpipe (LLVM 21.1.8, 256 bits)", vendor: 65541, device: 0, device_type: Cpu, device_pci_bus_id: "", driver: "llvmpipe", driver_info: "Mesa 26.1.2 (LLVM 21.1.8)", backend: Vulkan, subgroup_min_size: 8, subgroup_max_size: 8, transient_saves_memory: Some(false), limit_bucket: None }`. All ten samples used this adapter.
- hardware: snapshot load 2.974–9.931; actual GPU `software=false AdapterInfo { name: "Intel(R) Graphics (RPL-S)", vendor: 32902, device: 42891, device_type: IntegratedGpu, device_pci_bus_id: "0000:00:02.0", driver: "Intel open-source Mesa driver", driver_info: "Mesa 26.1.2", backend: Vulkan, subgroup_min_size: 8, subgroup_max_size: 32, transient_saves_memory: Some(false), limit_bucket: None }`. All ten samples used this adapter.

PSS depends on other mappings and processes on the machine. The preflight
live-artifacts record observes executable sharing for the existing engine
cohort; other shared mappings are uncontrolled. Venus shares wl_shm buffers
and Wayland/Mesa pages with the excluded Sway compositor. PSS splits these
pages between processes, so Venus process PSS reads low relative to the full
memory used to support its GUI. The existing snapshots give this separate
Sway diagnostic, in MiB with the same median and p05–p95 convention:

| Run cohort | n | Sway PSS before | Sway PSS after | Sway PSS delta |
|---|---:|---:|---:|---:|
| A | 10 | 22.636 (17.188–23.096) | 24.849 (19.162–25.298) | 2.203 (1.975–2.233) |
| B | 10 | 71.601 (71.238–72.081) | 59.476 (59.100–59.940) | -12.143 (-12.160–-12.118) |

The [per-sample Sway CSV](venus-client-memory-2026-11-08/sway-pss-diagnostic.csv)
is reconstructed from the same excluded Sway PID/start-time in each archived
before/after pair; swaybg is excluded from this diagnostic. Deltas are computed
per pair, then summarized. They include the warmed compositor's own allocation
changes and shared-page attribution, so a negative delta is possible. They are
not added to the native-client estimator and do not isolate missing shared GUI
memory.

The software PSS low tail is retained even though its RSS and anonymous PSS
ranges stay narrow. These deltas are process-memory attribution, not isolated
heap usage or a leak bound; each reported cohort retains all ten samples. The native GPU identity, requested render node, ICD, full cleared
environment, process identities and raw rollups accompany each sample.

Recorded artifacts:

- [Software result JSON](venus-client-memory-2026-11-08/result-lavapipe.json)
  and [native evidence](venus-client-memory-2026-11-08/evidence-lavapipe.tar.gz).
- [Intel hardware result JSON](venus-client-memory-2026-11-08/result-hardware.json)
  and [native evidence](venus-client-memory-2026-11-08/evidence-hardware.tar.gz).
- [Earlier reference JSON](venus-client-memory-2026-11-08/reference-memory.json).

The recorded harness is the release build of the frozen implementation tree
below, compiled with `cargo build --release --locked -p eon-bench` under
`nix develop`, `nice -n 19`, `CARGO_BUILD_JOBS=6` and
`CARGO_TARGET_DIR=/home/lucca/.cache/eon-agents/eon-bench-venus-work/dev-target`.
A wrapper supplied the landed `.16` Nix bench closure's pinned tools and
drivers; the recorded harness was not built by `nix build .#bench`. The
measured product is the accepted `.16` Eon 0.1.22 Nix candidate, built through
an out-link (store artifact `nx6lfzdx1wqdyph1w53bkpcf5rwxg3hp-eon-0.1.22`).
Product source is unchanged at this slice's base `0f3803b`; this slice changes
the harness. Final package delivery is checked separately. Harness/workload
processes are excluded from every measured set. The configuration disables
Anima and sets an opaque background (`background_opacity = 1.0`); diagnostic
trace capture is enabled to verify the actual GPU and terminal presentation.

Each result uses the existing memory suite schema 1. Counters contain before,
after and denominator for PSS/RSS/Pss_Anon. Every native sample was independently
reconstructed from its raw rollups, process identities, exclusions, viewport
and completion oracles, connection sets and actual adapter. The evidence
archives retain the native GPU/present trace excerpts and the full trace's
SHA-256. The original full traces and process logs are not included; the
archives retain the raw memory and verification evidence described above.
The result JSON records exact executable paths/hashes, tool versions and
build provenance. Implementation tree: `09865e889743c5ce204a05349aab4224a4784fae`.

The following rebuilds the current candidate and harness as Nix packages and
repeats the workload and method; it is not the build command used for the
recorded harness above:

```sh
nix build .#default --out-link candidate
nix build .#bench --out-link bench
bench/bin/eon-bench memory --eon candidate/bin/eon --subject eon \
  --renderer lavapipe --samples 10 --warmups 2 --max-load 64 --load-wait 0 \
  --out results-lavapipe
bench/bin/eon-bench memory --eon candidate/bin/eon --subject eon \
  --renderer /dev/dri/renderD128 --samples 10 --warmups 2 --max-load 64 --load-wait 0 \
  --out results-hardware
```

Defaults retained: 1000 ms settle and 300-second launch deadline. These are
memory measurements, not frame-time, input-latency or macOS footprint results.
