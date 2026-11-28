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
device and frame pass. Without arguments it shows fixed fixture frames. Given
`yazelix-venus`'s launch arguments (eon-yp6vg.7.9), it runs Venus's own
application with Rune as the native host. Building the entry requires the
non-default `candidate-rune` feature. The shipping `yazelix-venus` package
leaves the feature off, and the Nix graph check resolves it separately. Eon's
package also ships the entry on Linux and Apple Silicon macOS as the separate
`venus-rune-preview` package, which `eon window new --rune-preview` opens
(eon-yp6vg.7.4, eon-yp6vg.71; Linux also installs an "Eon (Rune preview)"
desktop entry). eon-yp6vg.54 removes the entry.

The candidate uses Rune's upstream `App::on_close_request` from anomalyco/rune#2024.
Venus defers native close requests to its workspace lifecycle policy. The
separate AppKit `App::on_quit_request` hook still routes application Quit to
that policy. Programmatic close and control EOF use the teardown path.

On Linux it requires `WAYLAND_DISPLAY` before creating the event loop. Rune
uses winit's Wayland backend with X11 disabled. On macOS both hosts use the
native event loop. The launcher below and the lavapipe check are Linux-only.
Run `nix develop -c eon-darwin-clippy` from the repository root to lint all
candidate targets for `aarch64-apple-darwin` after the feature-off workspace
pass. On Linux the candidate pass and focused invocations use the dev shell's
pinned Zig for `cc` build scripts such as `blake3` and `psm` (which assembles
`src/arch/aarch_aapcs64.s` with `-xassembler-with-cpp`). A non-empty
`CC_aarch64_apple_darwin` or `AR_aarch64_apple_darwin` overrides that tool's
default. On Apple Silicon the command uses the native C tools. The candidate
ships in the Darwin package with a `mac-proof` `rune-preview` lifecycle and
capture step (eon-yp6vg.71, native run pending post-landing `mac-proof`).

Each line on stdin advances one fixture step. After the last step, the first
step comes back as a fresh attachment. After a step's first presentation, the
entry prints `drawn <cycle> <step> <name>`. That line means the client
presented the frame, not that the compositor shows it yet.

`--host venus` draws the same steps with Venus's own windowed path and the
hosted renderer's fixed look. That host is the parent the native pixel proof
compares against.

With application launch arguments, the Rune host passes the configured
cursor tail to Venus's existing painter in `src/render/cursor_trail.rs`.
The locked Rune revision `c22cc2d459cc6b80a3f4ecf45b136eed1d205507`
supplies `Frame::dt` and `Compositor::animating` in
`crates/rune/src/compose.rs`. The latter keeps frames coming while the
tail moves. Blink still uses the application's `Tx::wake_after` deadline
in `crates/rune/src/el/tx.rs`.

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

### Measured popup header pilot

`venus-rune-candidate --fixture header-pilot --host rune|venus` opens the
private popup comparison at 1280 × 800 logical pixels. Set
`EON_VENUS_TEST_SANS_FONT` to the packaged DejaVu Sans regular TTF. Both
hosts shape the popup geometry with the same Venus font setup. Rune draws the
fitted title and upper outline through its public text and compositor APIs.
Use `EON_VENUS_TEST_HEADER_LOOK=notch|connected` to select the translucent
Notch or opaque Connected fixture.

Each stdin line advances the shared fixture. A completed presentation prints
`drawn CYCLE STEP NAME WIDTH HEIGHT SCALE FOLLOW_UPS`. The dimensions are
device pixels and the follow-up count is cumulative. A resize can produce a
receipt for the current step at the newly painted geometry.

For the Linux comparison, use a closure-only `FONTCONFIG_FILE` containing
the package's font directories and the preview wrapper's library paths.
Darwin uses the packaged font directories and native color emoji. Run the
mounted layout test with `eon-test --run-ignored only` and the ignored
`rune_header_pilot_outline_and_grid_match_venus` test on the headless Vulkan
or Metal adapter used for the grid proof.

### Generated active-terminal parity

The feature-on `eon-test` suite runs `rune_differential_noop` to check
deterministic traces, prepared state and replay. The GPU tests compare the
selected terminal's prepared state and exact RGBA bytes at every step. Traces
cover cell styles and widths, cursor blink, image placement and removal,
scale changes, pane selection, popup visibility, and zoom.

Run in the dev shell with packaged fonts and a headless adapter. On Linux,
set the Vulkan loader library path and lavapipe ICD as for the fixture probe.
On Apple Silicon use a native dev shell with Metal. The forced clip test
requires a real one-pixel difference and decodes both reported PNGs.

```sh
cargo test --locked -p yazelix-venus --features candidate-rune \
  --lib rune_differential -- --ignored --nocapture --test-threads=1
```

The default campaign runs 8 seeds × 16 steps. Seed 0 guarantees transition
coverage and the other seeds choose every operation from the generator. Set
`EON_VENUS_TEST_RUNE_DIFF_SOAK=smoke` for 2 × 64, or
`EON_VENUS_TEST_RUNE_DIFF_SOAK=1` for 256 × 64. Use the
`rune_differential_default_or_soak` filter to run only the campaign and a
20-minute watchdog for the full soak. Set `EON_VENUS_TEST_RUNE_DIFF_DIR` to
an existing scratch directory for the reduced failure's PNGs. An unset or
empty value uses the system temporary directory.

## AppKit event-path driver

`venus/tests/appkit_event_driver.rs` (`harness = false`, paired with Linux `eon-bench events` under `eon-33cww.4.2`) runs 2-process isolated AppKit event-path proofs (`static-echo`, `moving-caret-30`, `moving-caret-60`, `held-repeat`, `configure-storm`, `modifier-storm`) against both `run_venus()` and `run_rune_candidate()` using `NSTextInputClient`, `dispatch2` main-queue dispatch, `mac-proof`'s `CGEvent` helpers, and Venus's `EON_BENCH_TRACE` stream (`present_revision`, `rebuild_begin`, `rebuild_end`, `resized`, `input`, `input_end`, `input_rejected`, `key_released`). Shared verdicts live in `venus/tests/appkit_event_driver/judge.rs` and run everywhere via `venus/tests/appkit_event_judge.rs`. On Apple Silicon macOS, `mac-proof run` invokes it automatically via `nix develop --command cargo test --locked -p yazelix-venus [--features candidate-rune] --test appkit_event_driver -- --host <venus|rune> --orbit <yazelix-orbit> --mac-proof <mac-proof> --only <fixtures> --deadline 180` with `EON_APPKIT_PROOF_OUT=<dir>` and `EON_APPKIT_RUN_ROOT=/tmp/ev-<pid>-<step>`.
