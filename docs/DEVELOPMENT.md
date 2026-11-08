# Development

Eon owns orchestration, product configuration, component selection, updates,
and distribution. Orbit owns persistent terminal state and attachment; Venus
owns native presentation and input. Yazelix Nova remains a separate
product line.

Enter the pinned x86_64-linux development environment with `nix develop`.
It provides Rust/Cargo 1.96.1, formatting and lint tools, Git, `br` 0.7.2,
`bv` 0.25.2, the pinned Ghostty source, Zig, native libraries and fonts that
Orbit and Venus build and test with, and `eon-test`. The Beads binaries come
from hash-verified release archives and retain their full upstream licenses
under `share/licenses/<tool>/LICENSE`, including the OpenAI/Anthropic rider.

One Cargo workspace holds Orbit (`orbit/`), Venus (`venus/`), Eon Runtime with
EONW (`runtime/`) and Eon (`eon/`). `eon/crates/eon` owns executable assembly
and defaults; `runtime/` owns the runtime mechanisms behind `run(Inputs)` and
the canonical EONW package. `eon/crates/eon/tests/workspace_control.rs`
exercises the real Eon binary, aliases and self-launches.

Run the test suites with `nix develop -c eon-test`. It fetches locked crates,
then runs `cargo test --locked --offline` in the bubblewrap sandbox that
`nix/test-sandbox.nix` defines, so it is safe from inside a live Eon terminal.
Without arguments it runs the workspace suites, then the installer's, stopping
at the first failing suite; with arguments it passes them to `cargo test`, for
example `eon-test -p yazelix-venus`. It needs unprivileged user namespaces and supports
x86_64 Linux only. It is proved on a non-NixOS host; on NixOS the hidden `/run`
also hides `/run/current-system` and `/run/wrappers`. The Apple Silicon dev
shell has an unproven macOS branch without namespaces: it refuses while an Eon
supervisor of the same user runs, gives the suites a private HOME, TMPDIR and
runtime root, and skips the Linux-only installer suite.

## macOS proof harness

`nix run .#mac-proof -- remote --host USER@HOST --out DIR`, run on Linux inside
the checkout, proves a tree on an Apple Silicon Mac over SSH. It sends `git
archive` of HEAD, or of the candidate tree when the checkout has uncommitted
or untracked files, into a fresh `~/eon-proof/<run>/src` on the Mac. There it
runs `mac-proof run` from that tree and copies the evidence folder back into
`DIR`. `run` checks that the Mac holds exactly the sent tree. It builds the
aarch64-darwin package, checks Orbit's terminfo with macOS's own ncurses
(ORB-C10), lists the Darwin flake checks in `check-list` and builds their
shared inputs once in `check-inputs`. It then builds each check, skipping
checks whose inputs failed (or whose input status is unknown). If combined
derivation evaluation fails, paths are evaluated individually; checks that
still fail evaluation run their own build. It runs `eon-test` once per
workspace package. Each step has its own deadline, log and status. `--rev`
picks another commit, `--timeout` is the run's time limit on the Mac (12
hours by default; `remote` waits 30 minutes more) and `--dry-run` runs only
the source step. `remote --local DIR` stands this machine in for the Mac,
with `DIR` as its HOME.

The evidence folder holds `summary.txt` (one line per step: status, duration,
first failing line), `summary.json` (the same, with the system and tree),
`steps/NN-<step>.log` and the Linux side's `remote.log`; `mac-proof run --help`
lists the steps and fields. The tool exits 0 when every step passed, was planned
in a dry run, or was skipped for Yu; 1 when any step failed, timed out or was
skipped for another reason; and 2 when the round trip itself failed or the run's
outcome is unknown. Until the last step has run, the summaries' result reads
`incomplete`.

The Mac needs SSH key login with its host key already in `known_hosts` (the
tool runs ssh in batch mode), macOS 14 or later and Nix 2.20 or later,
installed once with sudo by the
[official installer](https://nixos.org/download/#nix-install-macos)
or Determinate Systems' installer; the tool enables flakes itself. Keep Nix's
macOS default `sandbox = false`: only unsandboxed Darwin builds are proved;
`eon-zuou` owns a sandboxed build. Nix uses the flake's Cachix substituter only
for a user named in `trusted-users`; otherwise the Mac builds everything. The
run keeps going if SSH drops, and the tool then prints the command that fetches
its evidence again. Otherwise its last line names the cleanup command:
`ssh USER@HOST rm -rf eon-proof/<run>`, then `nix-collect-garbage` on the Mac
to free the store paths it built; `eon-test` also fills the Mac's `~/.cargo`
with the locked crates. A Darwin build that fails mid-test can leave a
`/tmp/e<c>` directory; the `leftovers` step names any it finds.

Without a Mac, the hand-dispatched
[macOS proof workflow](../.github/workflows/mac-proof.yml) runs `mac-proof run`
on a GitHub-hosted Apple Silicon runner, with a 320-minute limit. The revision
must already be on GitHub, and pushing stays on the maintainer's request.
`gh workflow run mac-proof.yml --ref edge -f rev=<full commit SHA>` starts it
(checkout takes a short SHA for a ref name and fails), `gh run
list --workflow mac-proof.yml` finds the run, and `gh run download <run-id>
--name mac-proof --dir <dir>` fetches the evidence folder, which GitHub keeps
for 14 days. A run is candidate evidence, and suite proof for ORB-C14, but
never GUI proof.

`run` ends with a GUI scenario on the package it built, and `mac-proof scenario
--package PATH --out DIR` runs that scenario alone. It needs the tool's user
logged in to the Mac's screen; without a person at the Mac, turn on automatic
login with `sysadminctl -autologin` (FileVault off) and restart. It launches
full Eon with a private HOME, configuration and runtime root, then checks
version identity and Stop. With Screen Recording, the scenario shows a solid
backdrop below every window on the main display during its captures. With
Accessibility, which lets it post key events, it also dismisses the first tab's
picker, which takes a key, and drives Eon through the `eon` CLI, a driver in
its first pane and keys sent to Venus. That covers panes, tabs, focus, `eon
edit`, a pane's environment and size, the Files popup, Command+V, detach,
Orbit's refusal of an attaching client that is not Eon, `eon pane read` and
`eon pane send`, a Venus crash and supervisor adoption. With both
Accessibility and Screen Recording, captures of Venus's window also check its
background translucency over the backdrop, a Kitty image, zoom and the
Shortcuts dialog. Granting either permission takes a click in System
Settings. A step that lacks what it
needs is skipped and listed for Yu in `summary.txt`. A run whose only non-passing
steps are these skips passes and reports how many steps were left for Yu; each
step's status stays skipped. Command+C on a pointer selection, the Eon Bar's chips
(until `eon-agent-parity-a9g0.12` lets the CLI read them), VoiceOver, Kotoeri
and blur are always left to Yu. Retina is too, unless the main display runs at
scale 2, offers a HiDPI mode the step can switch to, or `mac-proof run` (CI and
the rented Mac) can use a temporary HiDPI virtual display. `mac-proof final`
never creates one on a person's own Mac; macOS keeps one ColorSync profile for
the virtual display's fixed identity, which `leftovers` logs.
`mac-proof scenario --help` lists the steps and what each needs.

The `app-bundle` step uses LaunchServices (`open`) to start the package's
`Applications/Eon.app` with the system-only PATH, private HOME/configuration/
runtime roots and no shell-startup marker. A fixture login profile exposes a
command only in `~/.nix-profile/bin`. The managed shell must resolve that
command and the candidate's packaged `eon` and `hx`; the step then types
`eon hx`, observes the Helix process, captures it when permitted and stops its
private workspace. The main CLI/driver scenario runs separately. Linux's
`macos-app` Nix check builds the icon/resources and tests the native launcher's
bounded PATH import; it proves no LaunchServices behavior.

The current automatic step and private guided test app supply `run` arguments;
neither proves the product bundle's no-argument Finder/Dock launch or a click
while Eon runs. `eon-mac-launch-kz4p.59` owns native reopen/Quit and
`eon-mac-launch-kz4p.60` owns the dispatcher and bundled Venus presenter,
including those missing lifecycle checks. A passing preliminary app step or
guided item does not close the bundle's EON-C7 proof gap.

### Yu's final check

Run `nix run github:Yazelix/eon/<rev>#mac-proof -- final` on the Apple Silicon
Mac. Grant the terminal app Screen Recording (window captures) and
Accessibility (automatic keys) in System Settings > Privacy & Security;
the command opens missing grants' panes and asks you to quit and reopen the
terminal app before rerunning.

On macOS 27 or later, macOS asks once whether your terminal app may "bypass the
system private window picker and directly access your screen and audio". The
check takes one throwaway screen capture at the start so the dialog comes
then; choose Allow. Screen Recording does not answer it; left open, it covers
Eon in every capture and the translucency check is skipped.

The automatic part captures whole screens, every connected display included,
and the desktop before launch: close or hide windows you would not send before
starting. The check asks you to hide desktop widgets (System Settings > Desktop
& Dock) and turn on Do Not Disturb (Control Center > Focus), then press Enter.
The end-of-run notification may be silent; the terminal bell still rings.
The archive replaces the home path with `~` and per-account temporary paths with
`/var/folders/<id>`. It replaces the account name with `<user>` in logs and text
files. The check's own `summary.txt` and `final.json` keep their wording, with your
notes and its errors redacted; other JSON files keep their keys. Names that match
status words or hashes stay unchanged, with a terminal warning. Captures (pixels)
can still show names: item 5's tab opens in HOME and its label shows the account
name. Look inside the archive before sending it.

The automatic part runs first and takes hours. Leave the Mac awake and
untouched until it rings: the GUI scenario types into Eon. The nine manual
items are Look, Second display (answer s to skip if you have only one), Typing
and input methods, Fonts, Keyboard, Yazi images, Agent, Copy out, and Finder
and Dock. The last item uses the private test app printed by the check:
restart it from Finder and the Dock, verify `which eon hx`, and open Helix
with `eon hx`. Remove this temporary app from the Dock afterward. Its wrapper
retains the check's private roots even after all its windows are stopped;
it invokes the candidate's actual app launcher and packaged composition.
`mac-proof final --help` prints each action and expected result. Eon uses the
default configuration with private state, while keeping your HOME and shell
environment so your logged-in Claude Code works.
During the check, Eon's Starship prompt hides the host and user names. A prompt
your own shell configuration sets replaces Eon's Starship prompt and shows as
usual; check the captures for host or user names before sending. The check keeps
Cargo's downloads out of HOME. At the end, its HOME report offers entries named
for Eon's components, CodexBar included, for removal with your consent; other
new entries are counted in the archive and listed by path only on the terminal.
If you already use Nushell, Atuin or Zoxide, the check's commands join their
existing history; the check removes only whole entries it created.

Each item waits for Enter, captures only Eon's largest window without changing
focus, then asks for pass, fail or skip and an optional note. Other windows may
cover Eon. The evidence folder defaults to
`~/eon-mac-final-<rev12>-<UTC date>` (with a numeric suffix if taken);
`--out DIR` chooses a fresh folder; its name is the archive's top folder and shows
in its paths, so choose one without your name. The archive is `<out>.tar.gz` beside it.
Look inside before sending it to Lucca. `--dry-run` walks the items without
launching or capturing on any system; it proves no Mac behavior. Ctrl-C
stops the check; a rerun starts over, and leftovers may need
`eon window stop all` with the private runtime environment recorded in
`manual/final.log`.

## Benchmark harness

`nix run .#bench -- <suite> --out DIR` runs a suite of `eon-bench`
(`eon/tools/eon-bench`), a development tool outside Eon's closure; `eon-test`
runs its tests. `libc` supplies PTYs, termios and the machine facts, whose
layouts differ by platform, and the in-tree `orbit-protocol` crate its Orbit
client's wire format; it has no other dependency. `eon-bench COMMAND --help`
documents each command, its flags and defaults.

`corpus` writes the benchmark corpora to `DIR/<id>.vt` with a
`DIR/SHA256SUMS` that `sha256sum -c` checks: `ascii`, `styled-light`,
`styled-heavy`, `unicode` and `cursor` (16 MiB each) and `cat64` (64 MiB of
the `ascii` generator). Each is generated from a fixed seed, ends on a whole
line or escape sequence, uses CRLF line ends (a PTY's ONLCR delivers them as
CR CR LF), and must match the length and SHA-256 recorded in `src/corpus.rs`;
the run fails otherwise. `cursor` addresses an 80x24 grid. `--only ID` writes
one corpus, and `eon-bench corpus --help` lists them with their hashes. Corpus
files are outputs; keep them out of the repository.

`write FILE --report REPORT` is the completion writer, run inside a terminal:
it discards queued input, writes FILE, asks for a status report (DSR,
`ESC[5n`) and stops its clock at the terminal's reply, so a terminal cannot
score by skipping work. Without a reply within 60 s it fails; either way it
restores the terminal's mode. REPORT gets the bytes and seconds.

`floor --layer pty` is the kernel floor: each corpus drained from a bare 80x24
PTY by eon-bench, in nonblocking reads that wait only when the PTY is empty,
while the writer writes it. Every sample must drain exactly the corpus, one CR
per LF and the query, and fails after 60 s of silence. Before each sample it waits while the
1-minute load is above `--max-load` (online CPUs / 4 by default), for at most
10 minutes (`--load-wait MINUTES`); `--cpus LIST` pins it on Linux. Two warmup rounds, then ten
sample rounds, each running every corpus once (`--quick`: one and three).
Throughput is MB (10^6 bytes) of corpus per second.

`orbit` is Orbit's throughput to completion. Each sample starts a fresh
release `yazelix-orbit serve` (`--orbit PATH`; `nix run .#bench` supplies the
flake's) at the grid (`--grid ROWSxCOLS`, 24x80 by default), whose child is
`eon-bench child`: it waits until its terminal reports the grid and the
harness says go, runs the workload, and holds the session open afterwards
until released, so the harness reads Orbit's processes before Orbit exits.
The harness attaches with the `orbit-protocol` codec (Hello, then a resize
to the grid), and a thread decodes and discards every message from then on.
Once the child has the grid and has sent any setup, and the client has read
nothing for 100 ms (at most 2 s, or the sample fails), `detached` mode closes
the client and waits until Orbit has dropped it, and `attached` mode starts
counting; it stops when the client has again read nothing for 100 ms, at
most 2 s after the workload. Both modes run by default, interleaved with the
workloads in each round. The workloads are the corpora, `vtebench-<name>`
for vtebench's twelve benchmarks, and `kitten-<name>` for the five
benchmarks of kitty 0.47.4's `kitten __benchmark__` (`--kitten-repetitions`,
its default of 100 unless set). A vtebench workload is a vtebench sample at the grid, generated by
running its scripts once with a controlling terminal of that size: a reset
and the setup, which the child sends untimed before the go, as vtebench
does, then the benchmark repeated to `--vtebench-bytes` (1 MiB, as
vtebench's `--min-bytes 1048576` in Orbit's earlier tables), which is timed;
its SHA-256 is recorded, not pinned, since it depends on the grid.
The bench package pins vtebench's scripts, the tools they call, and kitty's
`kitten` output. A corpus or vtebench sample is timed by the completion
writer; kitten times itself to its own status-report replies and reports its
MiB/s and time, from which the cell's bytes come. Counters per sample:
`attach_us` (the handshake, outside the time); attached, `setup_frames` and
`setup_last_revision` (read before the go and not counted after it),
`frames`, `frame_bytes`, `other_bytes`, `first_revision`, `last_revision`,
`revision_gaps` (a revision skipped: Orbit replaced a queued frame, which it
does when the client falls behind), `revision_repeats`,
`frames_after_reply`, `last_frame_after_reply_us`,
`reply_after_last_frame_us` (from the writer's monotonic stamps) and
`client_cpu_us`; on Linux, `host_cpu_us`, `worker_cpu_us`,
`host_context_switches` and `worker_context_switches`: CPU time and voluntary
context switches of Orbit's host and its Bend worker over the workload. The
table adds attached minus detached time and, per attached frame or MiB,
frames, frame size, host and worker CPU, and host context switches. Orbit is
not instrumented: every counter comes from the wire or `/proc`, and none
counts Orbit's turns directly.

The completion writer has no deadline on its write. A suite that runs it
inside a terminal owns the deadline: `orbit` kills Orbit by its PID when a
sample passes `--deadline` (300 s; 60 s with `--quick`), and the hangup ends
the child and the workload. On Linux Orbit, and the commands the harness runs
while preparing (`kitten --version`, the vtebench scripts, the environment
probes), also get SIGKILL when the harness dies, however it dies. Once samples start, SIGINT, SIGTERM or SIGHUP stops
`orbit` within about a second through the same teardown, removing the
sample's directory and keeping no `result.json`, and the harness then ends by
that signal; before then, and for a second signal, the default action ends
it.

A suite writes `DIR/result.json`, schema 1: the suite, its settings and start
and end times; `environment` (OS and version, CPU, online, pinned and
allowed CPUs, governor, kernel, 1-minute load at start and end, grid, tool
versions, and `build`: the running eon-bench's path, SHA-256 and profile,
with the Eon commit and dirty flag that `nix build .#bench` recorded, or that
a Cargo build's checkout holds when the run starts, marked
`checkout-at-run`); and one cell per workload, layer and subject with every
sample's seconds, load and counters, plus `stats`
(count, median, nearest-rank p95, min and max of the seconds, and the
matching MB/s, whose `p95` is the slow tail) and a summary of each counter.
Below 20 samples the p95 is the slowest sample.
Samples are the data; the summaries are recomputed from them. The `pty` layer
counts `pty_bytes` and `pty_reads`.

### Display-free memory

`nix run .#bench -- memory --out DIR` measures Linux PSS, with RSS and
anonymous PSS alongside it, at 80×24. Eon, tmux and Zellij are interleaved
by round: two warmup rounds, then ten samples per cell. `--quick` is smoke
only. `--subject eon|tmux|zellij` selects subjects; the wrapper supplies
Nix-pinned tools. Absolute `--orbit`, `--eonbase`, `--tmux` and `--zellij`
paths override them. `--eonbase` names the unwrapped executable so the harness
can supply its private Orbit and diagnostic presentation.

Each subject/round has fresh empty and filled launches, private HOME/XDG
roots and sockets, and bubblewrap PID/network namespaces with
`--die-with-parent`. Root is read-only; output and private tmp/HOMEs are
writable. Spawned commands also use parent-death guards. Namespace teardown
contains daemonized servers after failure, interruption or harness death.
No ambient runtime, profile or desktop is used.

| Measure | Included process set / calculation |
|---|---|
| `server-start` | Server plus one empty terminal. Eon includes its real terminal-mode eonbase supervisor, its Orbit host and Bend worker; reference engines live inside their servers. |
| `empty-terminal`, `filled-terminal` | For every subject, `(total at 50 − total at 1) / 49`, with server and terminal engines included. Eon has one supervisor/first terminal plus 49 independent Orbit host/worker pairs; it measures a terminal-engine cohort, not 50 Eon workspace records. |
| `client-50-filled` | After `(server + native frontend)` minus before `(server)`, with 50 filled terminals, for tmux and Zellij on private PTYs. **Eon's real Venus client is not yet measured**; `eon-benchmarks-xl2bp.18` owns that row. |
| `diagnostic-supervisor-alone` | Eon supervisor only, separate from the startup comparison. |
| `diagnostic-codec-server-attach` | Eon server/engine PSS change for a diagnostic canonical-codec client attached to all 50 hosts. It excludes the diagnostic frontend, verifies every reader and connection around sampling, and is separate from native-client comparisons. |

Eon trails tmux in the recorded startup, empty-terminal and filled-terminal
PSS comparisons. `eon-ypy0a` owns reducing those gaps; published results retain
the losses and name that fix bead.

Every workload child is excluded. The filled payload is exactly 10,000
numbered 72-column ASCII lines, CRLF-separated without a final newline. The
completion writer obtains a DSR reply before sampling; children verify their
PTY grid and hold the session open. The codec diagnostic checks all Orbit
viewports against the last 24 lines and records history counts. Native
clients are checked through client listings and the focused-viewport dump.
Tmux and Zellij use 10,000 history rows and no status/plugin panes; Orbit uses
its runtime byte budget. Equal emitted bytes do not guarantee equal retained
history. GUI/GPU memory and leak bounds are outside this suite.

Zellij is background-created with a layout. Its pinned background mode starts
at 50×50, so a temporary native client establishes 80×24 and detaches before
output. Setup allocations may remain; this is not a never-attached server.
Eon similarly uses its excluded diagnostic presentation to establish its first
grid, then detaches it before output.

`--settle-ms` defaults to 1000; `--deadline` bounds each subject launch at
300 s. Shared `--max-load`, `--load-wait` and `--cpus` controls apply. No quiet
window is reserved, and actual load is recorded beside each sample/snapshot.
Before isolation, `live-artifacts.json` records observed live executable-path
and backing-inode matches for the candidate Orbit/eonbase binaries (Nix may
hardlink identical executables under different store paths). Other mappings and background
applications remain uncontrolled. RSS sums double-count shared pages; PSS is
the primary comparison and anonymous PSS gives a second view.

`result.json` uses suite schema 1. For each metric (`pss_bytes`, `rss_bytes`,
`pss_anon_bytes`), sample counters `<metric>_before`, `<metric>_after` and
`denominator` define `(after − before) / denominator`; absolute measures use
zero before. Signed deltas are preserved. Seconds are collection time, not
throughput. `summary.md` reports MiB median and nearest-rank p05–p95 (with ten
samples, min–max), separating diagnostics and the unmeasured Venus row.
`evidence/` retains exact commands/configuration, completion reports, oracle
records, PID/start-time/parent identities, process sets, and every raw
`smaps_rollup`. Empty sets, missing/reused processes, missing fields and
failed/disconnected diagnostic readers fail the run. Incomplete runs retain
evidence and remove result/summary. These Linux measures follow the four
axes of Superlogical Rex's comparison, not its macOS `phys_footprint` values.

### Headless display launcher

`nix run .#bench -- headless --eon PATH --out DIR` checks a Nix candidate on
an isolated Linux headless Sway at 1280×800. Build the candidate with an
out-link, never an installed profile. The shared `launcher.rs` owns Sway,
D-Bus, a cleared subject environment, HOME, configuration, state, short
runtime roots and process identity checks. The response suite uses the same
launcher; terminal comparisons and Venus memory remain separate beads
(`eon-benchmarks-xl2bp.6`, `.18`).

The `headless` check reuses the memory suite's bubblewrap containment: private
PID/network namespaces, read-only host root, writable evidence and private
`/tmp` and `/run`. An interruption destroys the namespace, including daemonized
children. No ambient Wayland/D-Bus socket, activation token or Eon namespace
is inherited by a launched subject. Sway uses pixman for software mode;
Venus's GPU is recorded separately. `--renderer lavapipe` (default) selects the Nix-pinned Mesa Vulkan
ICD without hardware devices. `--renderer /dev/dri/renderD128` exposes only
that render node, uses a GLES Sway compositor to provide dmabuf, and rejects
software fallback. Mesa's EGL vendor file, GBM backend and DRI drivers are
supplied by the bench closure. Hardware mode pins `VK_DRIVER_FILES` to that
Mesa's Vulkan manifest directory; `result.json` records it (software mode
records the lavapipe manifest file). Available nodes vary by host.
The `headless` D-Bus uses a self-contained session policy without host service
activation and must be listening before Sway starts. Response keeps D-Bus's
`--session` configuration and startup ordering. The bench closure supplies
the compositor, bus and software renderer outside
the product closure. The smoke dismisses the startup picker through Eon's
`popup hide` CLI; it needs no input injector.

For an existing private artifact directory in `display_root`:

```sh
nix build .#default --out-link "$display_root/candidate"
nix build .#bench --out-link "$display_root/bench"
"$display_root/bench/bin/eon-bench" headless \
  --eon "$display_root/candidate/bin/eon" --out "$display_root/lavapipe"
"$display_root/bench/bin/eon-bench" headless \
  --eon "$display_root/candidate/bin/eon" --out "$display_root/hardware" \
  --renderer /dev/dri/renderD128
```

Use the active coordinator's heavy-command wrapper for builds and benchmark
runs. `--out` must not exist. `result.json` records the candidate, private roots,
Sway version, actual output mode, requested renderer and Venus's reported GPU,
load and startup/stop result. A presented terminal and the expected renderer
must be observed before Stop succeeds. Trace and process logs stay beside it.
This is a startup/cleanup check, without throughput, memory or latency claims.

### Native idle input response

`eon-bench response --eon PATH --out DIR` is an on-demand Linux/Wayland test,
outside the default `eon-test` run. It starts its own headless Sway, virtual
keyboard/pointer primer, D-Bus, HOME, configuration, state and two short runtime
roots. It never uses the calling desktop's display, seat, D-Bus or Eon root.
The candidate must be a Nix store executable; build it with an out-link so GC
cannot remove it between build and observation. The probe does not build,
install, refresh a profile or create machine stress.

Provide `wdotool` on PATH (it is not in the pinned nixpkgs). For example,
from the candidate checkout, with an existing private artifact directory in
`response_root`:

```sh
nix build .#default --out-link "$response_root/candidate"
nix build .#bench --out-link "$response_root/bench"
"$response_root/bench/bin/eon-bench" response \
  --eon "$response_root/candidate/bin/eon" --out "$response_root/idle" --samples 3
```

Follow the active machine coordinator's load gate and heavy-command wrapper for
the builds. `--out` must not exist. The probe creates private directories below
`/run/user/<uid>`; the Eon root must fit the existing 19-byte limit. No seed from
the person's Zoxide database is needed: the probe dismisses its startup picker
with the documented Escape action after the surface is presented. All terminals
run the benchmark's raw-input fixture, including its configured Git popup.

Cases cover tab clicks and Ctrl+Tab, pane clicks and Alt+J, column-terminal
clicks and Alt+L, popup opening, the Shortcuts header button and Alt+/,
an attention-inbox filter click, typing and an eight-key burst. A control
checks separate native ingress and consumption for the Shortcuts dialog key
and inside click, without leaking either into a terminal. Each tab, pane, popup
and column switch is followed by Escape (return keyboard focus from chrome,
without selecting or reattaching a terminal), then a typed key: only that fixture may
receive it, and its visible counter must increment.
The fixture draws its identity and response counter as high-contrast terminal
cells, beside the received key and count. The compositor capture must contain
the expected identity/counter; a model update or GPU submission alone cannot
pass. Local dialogs and expanded-column focus use image regions calibrated
against the opposite state.
Warm measurements capture only the response region to reduce copy and file I/O;
each sample records its screen rectangle. Calibration and the cold popup use
full-frame captures. The probe keeps setup inputs separate from measured inputs and never retries a
failed measured action. It continues collecting after a correlated visible
response with late or uncertain timing, and exits unsuccessfully at the end.
Missing, rejected or uncorrelated responses stop the run to keep later samples
from inheriting an unsettled action. Sway uses headless pixman rendering; Venus
selects its Vulkan adapter normally. The actual adapter and software flag are
recorded in result.json environment.gpu_adapters. A software-adapter run does
not establish hardware GPU, physical scanout, IME or stress performance.

The default is 100 repetitions per warm case and a 250 ms warm response budget,
overridable with `--samples` and `--bound-ms`. This is a test performance budget,
not a product latency guarantee or a runtime configuration default. Captures
bracket visible response time: an upper bound within budget passes, a lower
bound beyond budget is late, and an interval crossing the budget is inconclusive.
A cold popup observation runs once with a separate six-second budget. Each
burst records all eight send times and their common final cumulative response;
its upper bound covers every key in that burst. A missing response gets a
ten-second observation and thirty-second drain, then
fails with its last observed stages; absence alone does not prove permanent loss.

`DIR/result.json` uses the separate `eon-bench-response` schema 1. It records
settings, environment and artifact identities, every attempted sample and its
stage/capture timestamps, outcomes and median/nearest-rank p95/min/max visible
upper bounds. Absolute monotonic nanoseconds are strings to preserve precision.
`trace.tsv` retains the raw opt-in diagnostics; logs, reference images and failed
before/after PPM captures stay beside it. Fewer than twenty samples make p95 the
maximum. A failed or incomplete run exits 1 and retains its result; exit 2 is a
usage error. Compare cases separately; aggregate numbers mix different actions.

Four sensitivity runs must fail, detecting distinct causes:

```sh
eon-bench response --eon "$response_root/candidate/bin/eon" \
  --out "$response_root/drop" --samples 1 --sensitivity drop
eon-bench response --eon "$response_root/candidate/bin/eon" \
  --out "$response_root/missing" --samples 1 --sensitivity missing
eon-bench response --eon "$response_root/candidate/bin/eon" \
  --out "$response_root/delay" --samples 1 --sensitivity delay
eon-bench response --eon "$response_root/candidate/bin/eon" \
  --out "$response_root/invalid" --samples 1 --sensitivity invalid
```

`drop` records an explicit fixture discard (`fixture-dropped`). `missing`
records receipt but omits the visible effect without refusal (`missing-response`).
`delay` holds the response for twice the budget (`late`, or `inconclusive` if
capture timing straddles the budget). These faults arm only for the measured
typing case and change only the private fixture. `invalid` suppresses one driver send; it must report unobserved ingress
as a harness failure, without claiming a product drop. Production input routing
has no fault-injection switch. The retained
stages distinguish missing native ingress, explicit refusal, a received key with
no effect, and a late visible response. No trace, ambiguous visibility or a
trace sequence gap/loss is a failure, never proof that an input was dropped.

Venus, Runtime and Orbit emit diagnostics only when `EON_BENCH_TRACE` names the
probe's private nonblocking Unix datagram socket. The probe enables this only
for its own product processes. Orbit strips the variable at the PTY child
boundary; the private fixture receives its collector explicitly as an argument.
Input records contain coarse key/modifier/pointer classes and press/release
states, without key codes, typed characters or pointer coordinates. Other
records contain stage names, identities, counts, geometry and adapter metadata;
the fixture records its test counter. Captures include the private fixture's
visible test text and dialogs. Per-process sequences and ending/loss records
detect incomplete traces. Before Stop, every fixture must acknowledge a bounded
shutdown handshake carrying its final sequence and loss count; the collector
must have every record including its end. An
explicit exec handoff ends a producer before its replacement starts; lifecycle
validation keeps those sequences separate even when the PID stays the same.
Disabled hooks retain ordinary behavior. Instrumented idle measurements do not
establish tracing overhead or a performance improvement; stress profiles and
mitigations need separate scheduled proof.

## Sources of truth

- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — naming, ownership, and sequencing;
- [`docs/CONTRACTS.md`](CONTRACTS.md) — indexed product contracts and proofs;
- [`docs/DISTRIBUTION.md`](DISTRIBUTION.md) — composition and release policy;
- [`docs/REFERENCES.md`](REFERENCES.md) — routed primary and comparable sources;
- [`docs/VISUAL-REFERENCES.md`](VISUAL-REFERENCES.md) — discovery-only visual references;
- [terminal memory benchmark](benchmarks/eon-zlf-2026-09-08.md) — the measured
  Eon, eonbase, Foot, and Ghostty comparison; and
- [`CHANGELOG.md`](../CHANGELOG.md) — accepted user-visible chronology.

## Checks

[`components/eon-v5.json`](../components/eon-v5.json) is the canonical
distribution-neutral component graph. Validate it with:

```sh
cargo run --locked -p eon-manifest -- components/eon-v5.json
```

Nix builds Orbit, Venus, Eon and eonbase from the in-tree crates with the root
`Cargo.lock`. A workspace component's path must hold the expected Cargo package
at its manifest version. Any in-tree change selects a new generation through
Eon's [source-tree digest](ARCHITECTURE.md#in-tree-package-selection); a Cargo
checkout also hashes its docs and assets. Cargo needs every workspace member,
so Nix rebuilds Orbit and Venus after edits under `eon/` or `runtime/` too.

`nix build .#manual-bundle` assembles the unpublished Linux direct candidate
from the relinked tree described below. Its [manual delivery
contract](MANUAL-BUNDLE.md) records verification, host prerequisites, native
proof and publication limits. `nix build .#release` (`nix/release.nix`) adds
the static installer and one `SHA256SUMS` over both: the files the
[release workflow](INSTALLER.md#release-assets) attests and attaches.
`nix/linux-bundle.nix` owns archive assembly: the tree, an `eon` entrypoint
linked to the tree's launcher, and the receipts. `provenance.json` names the
glibc floor, the excluded store paths and the substitutes, and
`closure-paths.txt` lists the store paths the tree ships.

`nix build .#installer` builds the separate static Rust `installer/` workspace.
Its seven focused tests exercise real archive boundaries, filesystem
selection/removal, timeout child cleanup and CLI environment fallback;
`eon-test` runs them with the workspace suites. `nix/installer.nix` builds/tests the native musl artifact
and rejects a dynamic bootstrap. Signed TLS delivery and native active-work
proofs remain with `eon-prove-thin-linux-bundle-installer-8r7n`.
The [installer contract](INSTALLER.md) records target tools and recovery.

`nix build --no-link .#checks.x86_64-linux.bundle-relink` builds the
namespace-free tree the direct candidate ships. The separate
dependency-free Rust `relink/` workspace copies what Eon reaches without glibc,
Mesa and Wayland, points program interpreters at the x86-64 psABI path, makes
store RUNPATH entries `$ORIGIN`-relative and store symlinks relative, and drops
Venus's RUNPATH so the host GPU driver shares the host's gcc runtime. It then
checks that layout and gates every ELF on the glibc floor and on finding each
NEEDED library in the tree or among the declared host libraries. A substitute
ships a rebuild under a closure path's name: ncurses, built with
`nix/ncurses-termios-speed.h` so it needs no glibc 2.42 symbol.
`nix/bundle-relink.nix` owns that policy. The crate's tests run in that build
and with `eon-test --manifest-path relink/Cargo.toml`.

That build also replaces each makeWrapper script in the tree with a copy of the
static launcher from the separate `launcher/` workspace. `flake.nix` defines each
wrapper environment once: makeWrapper renders it for the Nix packages, and the
launcher compiles it in and applies it relative to the tree's location.
`nix build --no-link .#checks.x86_64-linux.bundle-launch` runs the tree from two
locations under bwrap, which stands in for a host without Nix. The stand-in host
glibc is the only store path visible, and the check fails if the tree looks up
any other besides the compiled-in data paths the tree stands in for. It also
compares each launcher's environment with makeWrapper's. It needs unprivileged
user namespaces inside the Nix build sandbox.

Build both installed products with `nix build --no-link .#default .#eonbase`.

Linux package checks use the sandbox's executable `/dev/shm` for ephemeral test
state, keeping lifecycle records and sockets off the builder's disk journal.
Sources and compilation stay in the ordinary build directory.

`nix build --no-link .#checks.x86_64-linux.helix-grammars` runs the pinned
Helix source-delivery check. It rejects built-in evaluator source fetches and
compiles every selected grammar through Helix's production builder. Grammar
sources use fixed-output fetchers with owned immutable archive URLs first and
upstream archive URLs second. `nix build --no-link .#helix-grammar-sources`
builds the complete source closure, including disabled grammars. The cache job
publishes and pins it separately from runtime closures so source outputs remain
available for substitution.

Every build loads the configuration corpus in
`runtime/crates/eon-runtime/testdata/config-corpus`: `config.toml` fixtures from
each release, each beside an `expected.json` of the resolved values and
warnings it must keep giving (on a mismatch the test prints what the fixture
resolves to). A release that adds or changes a configuration key adds or
updates its fixture. A renamed or removed key keeps its alias and fixture until
the declared migration floor (`MIGRATION_FLOOR` beside the corpus test in Eon
Runtime's `managed_environment.rs`, also stated in `CHANGELOG.md`) passes them;
then both go together.

Use Beads for implementation plans and deferred decisions:

```sh
bv --robot-triage
br ready
br show <id>
```

## Demo media

From the repository root, `nix run .#record-demo` records a scripted Eon
session on an isolated Wayland display. Install Codex CLI and sign in with
`codex login` first; the Agent popup uses that login for a fresh, read-only
session. The capture uses a soft focus wallpaper behind translucent terminals
and writes the GIF, MP4, and PNG poster to `assets/demo/`.

`nix run .#record-demo -- features` records the clips and stills behind
eonterm.com's Features page into `assets/demo/features/`, as web-encoded MP4
clips and WebP stills. The clips and the Eon Bar still show the live Codex and
Claude quota chips, so sign in to both first. The recorder reads those logins
in place through `CODEX_HOME` and `CLAUDE_CONFIG_DIR`. A Claude rate limit
hides or ages those chips, so re-record when it clears.
