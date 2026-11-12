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
environment to append one JSON line per successful presentation. It is off by
default, reads the environment once, and adds no product configuration or mode.
Use a separate regular file for each Venus process. No terminal text is recorded.
This is a development diagnostic, not a stable product file format.

Version 1 (`v: 1`) records:

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

All timestamps share `EON_BENCH_TRACE`'s rustix monotonic clock domain; durations
are nanoseconds. These are elapsed host intervals, not CPU accounting, GPU
completion, compositor display time or panel latency. Frame reduction, canonical
scene construction, rendering and presentation retain their normal behavior.
No row is emitted for a deferred, occluded, recovered or failed presentation.

A bounded queue sends rows to a buffered writer without waiting for disk on the
render thread. Clean shutdown drains and flushes it; crashes can lose buffered
rows. Open/write errors report once on stderr and stop logging. Check for errors,
nonzero `dropped`, and missing rows before using a log for measurements. The
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
