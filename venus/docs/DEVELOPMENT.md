# Development checks

## Architecture and evidence

- [`docs/CONTRACTS.md`](CONTRACTS.md) indexes Venus subsystem behavior and proof.
- [`docs/REFERENCES.md`](REFERENCES.md) records the exact architectural
  evidence used by the implementation gate.
- [`docs/CRATES.md`](CRATES.md) records the measured dependency selection,
  owner seams, and rejected alternatives.
- The [memory comparison](benchmarks/venus-memory-2026-09-08.md) records
  the cell-buffer allocation savings and their environment limits.

```sh
cargo fmt --check
cargo check --locked
nix develop -c eon-test -p yazelix-venus
cargo clippy --locked --all-targets -- -D warnings
```

## Frame timing diagnostic

Set `EON_VENUS_FRAME_TIMINGS=/absolute/path/frames.jsonl` in Venus's launch
environment to append a version 2 JSONL stream. It is off by
default, reads the environment once, and adds no product configuration or mode.
Use a separate regular file for each Venus process. No terminal text is recorded.
This is a development diagnostic, not a stable product file format.

A `kind: "start"` record precedes the first successful presentation and records
the process `pid` and the selected device's `adapter` identity (including
software status, backend, vendor, device and driver). Each `kind: "frame"`
record contains:

| Field | Meaning |
| --- | --- |
| `revision` | Latest frame applied to the selected terminal; null before a frame exists. Repeated revisions include cursor and chrome redraws. |
| `receive_ns` | Monotonic receive time captured before transport queue admission for that frame, including wheel/scroll viewport replies; null when unavailable. Coalesced frames retain their own receipt. |
| `present_ns` | Monotonic time after the native submit/present calls return. |
| `scene_ns` | Canonical scene construction across the window's current terminal models since the previous successful presentation, plus this presentation's workspace/dialog scene construction. Reused scenes and focus changes do not charge previously presented work again. |
| `render_ns` | Host elapsed time in the renderer before submission, excluding surface acquisition. |
| `rebuild_ns`, `prepare_ns` | Portions of `render_ns` spent rebuilding draw inputs and preparing dynamic cursor/text resources (including glyph-atlas retries). |
| `acquire_ns` | Surface texture acquisition, kept separate because it can wait. |
| `submit_ns` | Host elapsed time for command completion, queue submission and native presentation calls. |
| `full_rebuild` | Whether this presentation attempt rebuilt the cached window draw inputs. |
| `dropped` | Cumulative rows dropped because the bounded writer queue was full. |

Every record has `v: 2`. Clean shutdown appends `kind: "end"` with the producer's
total successful `presentations` and final cumulative `dropped` count, including
a dropped final frame. Consumers must require one start, one footer, zero drops
and exactly `presentations` frame records. A missing footer invalidates the log.
There is no version 1 compatibility path.

All timestamps share `EON_BENCH_TRACE`'s rustix monotonic clock domain; durations
are nanoseconds. These are elapsed host intervals, not CPU accounting, GPU
completion, compositor display time or panel latency. Frame reduction, canonical
scene construction, rendering and presentation retain their normal behavior.
No row is emitted for a deferred, occluded, recovered or failed presentation.

A bounded queue sends rows to a buffered writer without waiting for disk on the
render thread. Adapter identity is captured once, only on the enabled path.
Clean shutdown queues the footer, drains and flushes it; crashes can lose buffered
rows. Open/write errors report once on stderr and stop logging. Check for errors,
nonzero `dropped`, and missing start/footer/rows before using a log for measurements. The
diagnostic itself has overhead; compare enabled and disabled runs in a quiet slot.
Relative paths resolve against Venus's working directory; an absolute path avoids
ambiguity. Draining the writer on clean exit can delay shutdown if its filesystem
stalls. Use local disposable storage for measurement.

## Native checks

The application regression also needs an isolated native Wayland display and
Vulkan renderer, so it runs outside `eon-test`, which hides every display.
Point `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` at that display, and `TMPDIR` at
disposable proof storage, then run:

```sh
timeout 30s cargo test --locked --bin yazelix-venus live_output_keeps_application_input_admitted_before_repaint -- --ignored
```

The quit regression runs the same way. It drives the shortcut chain and workspace
handling: the quit prompt is redrawn and described again when only the busy list
changes, Enter queues one Stop, and a second Enter while it is in flight reaches
no terminal and queues nothing:

```sh
timeout 30s cargo test --locked --bin yazelix-venus quit_prompt_refreshes_and_a_stop_in_flight_holds_enter -- --ignored
```

The image regression runs the same way. It checks that oversize image uploads are
refused without failing the renderer, then shows a test image beside a refused image's
marker in a pane, in a popup and under a notice for 2.5 seconds each, so a capture of
that display can confirm the band:

```sh
timeout 60s cargo test --locked --lib images_draw_in_panes_and_popups_and_refusals_keep_the_renderer -- --ignored
```

## Rune candidate entry

`venus-rune-candidate` is the private Rune host candidate (eon-yp6vg.7.7). It
draws the canonical Venus grid with the hosted renderer inside one Rune
device and frame pass. It shows fixed fixture frames, never a live terminal,
and it has no input. Only the non-default `candidate-rune` feature builds it,
so no package, test or CI root contains it. eon-yp6vg.54 removes it.

On Linux it refuses to start without `WAYLAND_DISPLAY`, before any event loop
exists: the Rune pin compiles winit's X11 backend. On macOS both hosts use the
native event loop. The launcher below and the lavapipe check are Linux-only;
the candidate's first Darwin build and proof belong to eon-yp6vg.57.

Each line on stdin advances one fixture step. After the last step, the first
step comes back as a fresh attachment. After a step's first presentation, the
entry prints `drawn <cycle> <step> <name>`. That line means the client
presented the frame, not that the compositor shows it yet.

`--host venus` draws the same steps with Venus's own windowed path and the
hosted renderer's fixed look. That host is the parent the native pixel proof
compares against.

Build it into a directory of its own:

```sh
nix develop -c cargo build --locked --release -p yazelix-venus \
  --features candidate-rune --bin venus-rune-candidate --target-dir DIR
```

Like shipping Venus, it needs the runtime libraries and fonts that the
package wrapper sets, and the dev shell does not set all of them. This
launcher takes them from the shipping wrapper of the installed `eon` on
`PATH` (inside an Eon terminal, Eon's own tool directory comes first, so run it
from a desktop session or with the profile's `bin` first on `PATH`), then runs the
candidate. A desktop entry's `Exec=` can run it:

```sh
#!/usr/bin/env bash
# venus-rune-candidate.sh DIR/release/venus-rune-candidate [--host rune|venus]
set -eu
eon=$(readlink -f "$(command -v eon)")
venus=$(sed -n "s/^export EON_VENUS='\(.*\)'$/\1/p" "$eon" | head -n 1)
[ -n "$venus" ] || { echo "no EON_VENUS in $eon" >&2; exit 1; }
eval "$(grep -E '^(export |LD_LIBRARY_PATH=|XDG_DATA_DIRS=)' "$venus")"
exec "$@"
```

The native check of the candidate needs a real adapter and the packaged fonts.
On lavapipe it compares every fixture step drawn through the production
compositor with the parent's windowed recording, at scales 1 and 2. It also
checks an inset slot cut by a clipping ancestor, painter order, and a refused
frame on a second device. Set `EON_RUNE_PROOF_DIR` to keep every image pair:

```sh
VK_ICD_FILENAMES=<mesa>/share/vulkan/icd.d/lvp_icd.x86_64.json \
  timeout 600s cargo test --locked -p yazelix-venus --features candidate-rune \
  --lib rune_hosted_grid_matches_the_venus_parent -- --ignored
```
