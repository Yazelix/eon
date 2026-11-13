# Using Eon

## Supported environment

Eon is in alpha. v0.2.0 is the planned first beta; its cut requires explicit
release approval. Its prepared download commands require a published GitHub
release.
On Linux it installs without Nix or through Nix. The
supported product target is **x86_64 Linux on native Wayland**. From v0.2.0, Apple Silicon Macs
with macOS 14 or later run Eon as a preview; the
[README](../README.md#status-and-limits) lists its known issues.

Before installing, you need:

- on Linux, a native Wayland session; and
- for a Nix install, Git and Nix with flakes enabled. The Nix-free Linux
  install has its own
  [host prerequisites](MANUAL-BUNDLE.md#execution-and-host-boundary).

Venus draws with `wgpu` on Vulkan: a hardware adapter uses the GPU, and Mesa
lavapipe can render in software.

Before Eon moved into one repository, the accepted runtime proof used COSMIC
with systemd and Mesa on Intel hardware. This repository's installed checks so
far run on a private headless Sway, plus a brief COSMIC launch check; the full
COSMIC desktop proof awaits re-proof here. Eon does not require systemd or cgroup delegation, but non-systemd use and
proprietary NVIDIA remain unproved.

From v0.2.0, Apple Silicon macOS is a preview: it installs through the same
Nix channel, makes no stability promise and is not full support.
[EON-C7](CONTRACTS.md#eon-c7--native-linux-wayland-and-apple-silicon-macos)
records what is proved there, and the README lists its
[known issues](../README.md#status-and-limits).
X11, Xwayland, ARM Linux, Intel macOS, macOS direct bundles, Home Manager installation,
background updates, and release automation are unsupported. Restarting the
machine ends live terminal process state.

## Install and start

On Linux, the [README](../README.md#linux-without-nix) and the
[installer guide](INSTALLER.md#install-a-release) install Eon without Nix.
With Nix, install the latest release:

```sh
nix profile add --refresh github:Yazelix/eon/main
```

`main` always holds the latest release. To get fixes before they are released,
install `github:Yazelix/eon/edge` instead; it gets every change as it lands.
Tags such as `github:Yazelix/eon/v0.1.16` name one exact release and never
update. See [Update](#update).

Run `eon` to start. `eon versions` shows the generation of the in-tree build and
each exact external component revision; Orbit, Venus and Eon Runtime
report `workspace`, the Cargo workspace they are built in.

Fresh sessions first show a three-second random Anima animation after the
window is ready. Press any key other than the style-browsing keys to dismiss it;
set `[anima].enabled = false` in `config.toml` to skip it, or start with
`eon --no-anima` to skip it once. Quick search then opens over the first tab's
Files. `eon --dir DIR` skips both the animation and quick search: the first tab
opens a terminal in DIR. When other Eon windows still run, a new
window's first quick search says how many in its header, names up to three,
and says how to see and quit them; `eon window ls` lists them, and
`eon session --json` carries them as `running.others`. Press Enter to use a
Zoxide history match, or Tab to move Files there and browse with Yazi, where
Alt+Enter uses a folder (see
[Eon Bar and tabs](#eon-bar-and-tabs) for every key). New tabs start in your
home directory. Escape closes quick search and opens a terminal there, keeping
the tab. Close the tab with Alt+Shift+W if you no longer want it. The initial
session uses the directory where Eon was launched.

A terminal starts after choosing a directory or closing quick search. Closing the last
tab quits the window and stops its terminals; in a window with several
sessions, it closes only that session. By default Eon asks first, listing the
terminals that stop, and the question names the session while others remain.
Closing the Eon window, with the desktop's close-window key, the window's
close button or, on macOS, Command+Q, quits it as Alt+Shift+Q does (see
[Stop terminals](#stop-terminals)). To leave its terminals and PTY commands
running instead, press Alt+Shift+D: it detaches the desktop without asking. Each time, on Linux, Eon shows a desktop
notification naming the window and how many terminals still run, with a Quit
button where the desktop supports notification actions. Quit asks first and
then stops them. An eonbase window has no Quit, so closing it detaches; so does
closing a window whose supervisor does not answer, or that failed to stop.
Logging out detaches the window on a desktop that ends the session without
closing windows first, such as GNOME or COSMIC. A desktop that closes each
window at logout, such as KDE Plasma, quits an idle Eon window instead and
shows the prompt on a busy one; there, press Alt+Shift+D before logging out to
keep its terminals.
Reconnect with:

```sh
eon window attach
```

Press **Alt+/** inside Eon to open the native Shortcuts dialog. It includes the
fixed bindings and every enabled popup binding. Escape, Alt+/ or a
click outside it closes it.

Window labels use the platform's modifier names: on macOS, Option replaces
Alt and Control replaces Ctrl, including header tooltips and the viewer's
dismissal hint. Configuration keeps the physical-key spelling described in
[Configuration](CONFIGURATION.md#popups).

### Update

Check whether your install has an update:

```sh
eon update --check
```

It finds the installation the `eon` you ran came from, reads its source from
Nix or its managed Linux prefix, asks GitHub where that source is now, and prints the owner, the source,
the installed build, whether an update exists, and whether Eon may update
that installation, with the command to run. The check changes nothing;
`--json` prints the same facts as one
object (EON-C41). It checks only when you run it.

- A `main` or `edge` profile entry gets an answer from GitHub. A newer
  revision on the same version is still an update. Without network, or under
  GitHub's rate limit, the check fails (exit 1); it never reports a guess.
  Nix updates keep the installed `default` or `no-helix` package; to switch,
  install the other package separately.
- A tag or commit stays pinned and reports how to follow `main` instead.
- Home Manager, NixOS and nix-darwin installs are updated through their own
  configuration, which the check names.
- The check knows the installation from the path that started `eon`, such as
  `~/.nix-profile/bin/eon`. Inside an Eon terminal, `eon` is that window's
  own build, and `nix run` or a local build belongs to no installation; there,
  name it: `eon update --check --profile ~/.nix-profile --entry eon`, with
  absolute paths.

For an installer-owned Linux prefix, use `eon update --check --prefix /ABS/PREFIX`
or run the selected prefix's `bin/eon update --check`. This discovers published
numbered official releases and the exact Linux archive URL, excluding drafts
and prereleases. Matching version, archive URL and digest metadata means
unchanged; equal versions with different artifacts refuse, and lower releases
are never offered as upgrades. Network and malformed-metadata failures exit 1.
Manual extractions remain unmanaged. Discovery verifies no signature or archive
bytes. Install through that prefix's bundled installer after independently
provisioning the signer described in the
[installer guide](INSTALLER.md#installer-inside-a-payload):

```toml
[updates]
bundle_prefix = "/home/me/.local/share/eon/install"
allowed_signers = "/home/me/.config/eon/allowed_signers"
```

Run `eon update --install`, or pass `--prefix /ABS/PREFIX` explicitly.
Commands' **Install update** and the Window menu's **Updates** route use the
same configuration and confirmation. Eon uses only the installer inside the
selected payload that owns the running executable; no PATH fallback or fetched
bootstrap. Missing trust refuses with setup instructions. The installer verifies
OpenSSH-signed checksums, the archive hash and the payload's URL version before
selection. Old payloads and running terminals remain in place; rollback stays
an explicit installer command. `--prefix` replaces any configured Nix target
and excludes `--profile`/`--entry`.

Only builds from this release on carry the source receipt that ties an
installed build to its revision. An older build reports its provenance as
unknown: it is never current and never shown an update as available until it
is upgraded once.

Update a validated `main` or `edge` profile entry with upstream or Determinate
Nix 2.21 or later. Lix and other implementations are unsupported; integer and
absolute-path entry names are refused because Nix can interpret them as selectors:

```sh
eon update --install --profile ~/.nix-profile --entry eon
```

Eon displays the exact profile, entry and source before asking. A decline or
EOF cancels. For an agent or JSON caller, use `--yes` with that explicit target
or configure `updates.nix_profile`/`updates.nix_entry` or `updates.bundle_prefix`.
`--yes` always requires an explicit or configured target, even at a terminal;
a discovered target alone is refused.
Interactive confirmation requires stdin to name the controlling terminal. If
its device cannot be opened or its identity verified, Install fails with
`not-confirmed` without reading an answer.
A custom profile or
renamed entry uses its actual path/name. Only that entry is upgraded on its
original ref; a failed build is reported with its actual selected state.
An already current eligible entry succeeds unchanged. No channel switching,
remove/add, retry, garbage collection or live adoption runs.

Nix and installer progress goes to stderr; JSON stdout contains one final result, including
`selection` with the actual selected build/source after failure or cancellation.
Ctrl+C, SIGTERM, SIGHUP and SIGQUIT terminate only helper-owned children,
with a fixed five-second grace;
cancellation cannot promise rollback. Eon serializes its own operations per
resolved installation and rechecks its displayed source/entry/generation immediately
before mutation. Nix owns selection and its transaction lock; an external writer
can still race after that recheck. A detected change before mutation is refused
with `target-changed`; a source change observed after Nix ran is a failure with
`source-changed` and the observed selection. A changed unrelated entry also fails
selection read-back.

For builds whose provenance is unknown, update manually once with
`nix profile upgrade --refresh eon`. Then use the reported profile launcher for
fresh launches. Windows already running keep their build, terminals and layout;
installation never stops or restarts them. A tag install never updates; move it to `main` once with:

```sh
nix profile remove eon
nix profile add --refresh github:Yazelix/eon/main
```

### Reload window configuration

After editing `config.toml`, focus the window and run:

```sh
eon window reload ID
```

Use the ID or prefix from `eon window ls`. Reload rereads presentation settings,
including `[panels] heading`, and replaces Venus even when the build is
unchanged. It also selects the requesting CLI's Venus after an upgrade when
its protocols match the running window. When EONW differs, the command runs
with the window's recorded Eon build and requests that build's Venus, retaining
its reload behavior and supported configuration. Reload does not upgrade the
window's supervisor or EONW. The supervisor, terminals, running
commands, layout and selected tab/pane/popup survive. Venus-local view state,
including dialogs, zoom and terminal text selection, resets; see
[EON-C11](CONTRACTS.md#eon-c11--runtime-generations-and-presentation).

Invalid configuration refuses before replacing Venus. If the replacement fails
startup admission, Eon restores Venus with its last accepted launch arguments
and reports failure. Reload refuses when the window lacks keyboard focus,
when the executing build's Venus protocols differ from the window's, or when
the requesting Venus is outside the immutable
Nix store. `--json` reports `window`, `generation` and the presenting `venus`
path on success, together with `"accepted":true`, `"replaced":null` and
`"config_reloaded":null`. `accepted` confirms that the supervisor accepted
the request. Both null values mean the reply cannot confirm whether Venus
was replaced or configuration was reread; they do not mean false. EONW v42
does not report those outcomes. Failure still returns an error object.
The text reply acknowledges the request, reports the presenting path and
says replacement and config rereading are unconfirmed. Quota polling follows
the accepted `[bar] widgets`; a failed reload retains its previous selection.

Popup definitions added while the supervisor is running still need a fresh
window. Supervisors started before the same-build reload fix (`eon-u199d.3`)
accept without rereading config when the Venus path is identical. Move
those windows to a fresh window once to use the fix.

### Stop terminals

Click the Eon icon at the top left for the **Window menu**: this window and
its terminal count, other running windows, Detach and Quit. With keyboard
focus in the bar (F6), Tab reaches the icon and Enter opens it. Use arrows or
Tab to choose, Enter to activate, and Escape to close; pressing outside also
closes it. This window's Quit asks while a terminal is busy; another window's
Quit always asks first. The icon keeps working with hints and the sidebar off,
and stays reachable while tabs overflow; only a tiny width hides it. The session pill opens
the Sessions picker, and the bottom version label names the supervising build.

To inspect or stop work explicitly:

```sh
eon window ls
eon window stop ID
eon window stop previous
eon window stop all
```

`eon window stop` shows each window's live terminal identities and asks for
confirmation, naming the window by the ID `eon window ls` shows. `previous`
stops every window that another Eon build started, as after an upgrade, and
`all` stops every window, the default one included. Detaching a window keeps
its terminals; stopping it ends them. To end the window you are in, run
`eon quit` in any of its terminals, press Alt+Shift+Q or close the window, or
open Commands (Alt+Shift+M) and choose Quit this window, which runs it. It
stops only that window. It asks first only
while another of its terminals runs something other than its idle shell, and
names those terminals, as in `Quit this window? It stops 3 terminals; 1 still
running: p2  cargo build. [y/N]`; while nothing runs it quits without asking.
Alt+Shift+Q and closing the window (and, on macOS, Command+Q) follow the
same rule with a prompt in the window that names the busy terminals: Enter
quits, Esc cancels, and Alt+Shift+D detaches instead; closing again leaves the prompt open. If a quit
does not finish, the window says `Quit did not finish; press Alt+Shift+Q to
retry.` and `eon` commands that reach it say to retry `eon window stop ID`. A
terminal started for an explicit command, such as `pane create -- COMMAND`,
counts as running, and so does the Agent popup; the Files and Git popups
never do. Eon does not see a shell builtin or a background job of an idle
shell. Outside Eon, `eon quit` quits the only running window, asking the same
way with `Quit window NAME?`; when several run, it lists each by its ID
and name with the `eon window stop ID` command that stops it, or
`cannot stop: REASON` when this `eon` cannot stop it, and stops nothing. The
listing includes each window's state and live terminal count; an
uninspectable count is `unknown`. `eon quit --all` quits every window, asking
for each as `eon quit` does.
At a terminal, any answer but `y`, `Y`, `yes` or `YES` cancels
and exits 0. Without a terminal, a piped yes still confirms; any other answer,
including end of input, exits 1 with `not-confirmed` and sends no Stop.
Previous generations that use the current EONW can be stopped
through their own supervisor even when the current desktop cannot attach to them.
A generation from a build with another EONW version is listed as incompatible;
`eon window stop`, `eon quit` and `eon quit --all` stop it with the Reserved
Stop, one message that every Eon build from 0.1.21 on answers, whether or not
that build is still installed. In human mode they always ask first, as in
`Quit window 3f9a? It runs another Eon build, so its terminals are
unknown. [y/N]`, because this `eon` cannot see its terminals. A window of
Eon 0.1.20 or older predates the Reserved Stop: `eon window ls` shows its
Stop as unavailable, and its Stop fails with `stop-unavailable` and says to
quit it from inside its window, with Alt+Shift+Q or `eon quit` in one of its
terminals. Each generation
records its own build's `eon`, and `eon window ls --json` lists it as the
`executable` of a window this `eon` cannot present. `eon window attach ID`
presents such a window by running that build's `eon` as `window attach ID`,
with `EON_ATTACH_DELEGATED` naming the generation. The marker must name the
generation of the `eon` that reads it, in a window that runs it; any other
exits 2 and sends nothing, and Eon never passes the marker to what it starts.
An Eon 0.1.18 or older `eon` takes another form, which this release rejects,
so it cannot attach a window of this release. Generations started before Eon
recorded it need that build's `eon` by hand.
The runtime extraction changes the component graph and selects a new generation.
Reattach earlier work with its retained exact Eon or eonbase executable.

## Session and quick search

Each session has a persistent ID and an optional name. `s1`, `s2`, … are local
to one window. Names may repeat, renaming keeps the persistent ID, and a tab's
directory does not identify its session. Detailed inspection prints
`persistent_id`; `session ls --json` includes it beside each local `id`.
Recovered live terminals remain usable by their local handle while registration
is pending or failed. Their `persistent_id` is null, and nullable
`registration_error: {code, detail}` exposes any failure in inspection and
session listings. Venus shows that failure without disabling terminal input.
Confirmation emits `session-registered`; a failed registration emits
`session-registration-failed`. `--id` never matches an absent ID.

Use `eon session --id ID --json` to inspect a member, or
`eon session switch/rename/close --id ID …` for the corresponding operation.
Pass the full ID instead of a positional handle/name. Selection stays within
the addressed window (`--window ID` keeps its existing meaning); an identity
registered elsewhere is missing here and is never joined or restored.

`eon session registry [ID] [--json]` reads this user's host registrations
without resolving a caller or window or starting a supervisor. JSON is an
array sorted by ID, or one object for a specified ID, containing only
`persistent_id` and nullable `name`. A missing registry lists empty; a missing
ID exits 3. Window and generation flags are invalid for this command.
Registration does not mean that terminals are live or recoverable.
The registry belongs to this user's configured state directory
(`$XDG_STATE_HOME/eon/sessions`, otherwise `~/.local/state/eon/sessions`);
different state roots select independent registries.

Creation validates its inputs, commits the registration, then starts terminals.
Success retains `created: sN` and adds `created_persistent_id`. A failed start
after registration reports that persistent ID and retains it without adding a
member. A registry failure starts no fresh terminal. `registry-busy` exits 1
and means no identity was allocated or record published for that request.
Registry I/O does not block inspection, attention processing or Stop.
Startup and recovery retry temporary registry contention on the worker for up
to two seconds. Interactive create/rename still return busy immediately;
registry list/lookup share a read lock. Missing state-directory resolution or
worker setup also leaves recovered work usable with its diagnostic.
An uncertain commit or lost reply
needs inspection; do not automatically resend a new-session request. Replay
protection lasts only while the request remains in the supervisor's recent
256 mutations and that supervisor has not restarted.

Popup (`uN`), pane (`pN`), tab (`tN`) and session (`sN`) IDs never name a
different object after same-boot supervisor recovery. Read the recovered IDs
from `eon session --json` or `eon session ls --json`; old targets fail rather
than acting on a successor. Normal fresh launches start at 1. A launch that
finds the generation's lifecycle lock already present uses a separate numeric
range, so IDs can be long after recovery or an earlier failed launch attempt.
Terminal IDs (`terminal-N`) and persistent session registrations are separate.

Renaming or clearing a name persists before memory, events and success. A
post-publication sync failure reports uncertainty, retains the last confirmed
in-memory name, and rereads disk under the lock on the next change. Close,
natural exit and empty-session cleanup retain registrations. Disconnect and
reconnect keep the current mapping; supervisor restart recovery gives its
merged session a new ID after confirmation, while old registrations remain
queryable. Registry failure never hides or stops those recovered terminals.

A **session** is the whole collection of tabs and panes. A **tab** groups
panes around its launch directory and shows them in one to four side-by-side
**columns**. Each column stacks its panes and expands one. A **pane** is a view
of a persistent **terminal**, whose process, PTY and terminal state outlive the
window. Every column's expanded terminal stays live; keys go to the selected
one, and clicking another column's terminal selects it.

| Shortcut | Action |
|---|---|
| Alt+/ | Open or close shortcut help |
| Alt+Shift+B | Show or hide the bottom shortcut hints |
| Alt+Shift+H | Show or hide the agent sidebar |
| Alt+G / Alt+Shift+G | Jump to the next or previous agent needing attention |
| F6 | Move keyboard focus: terminal, agent sidebar, tabs, panes |
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+Shift+1 … Alt+Shift+9, Alt+Shift+0 | Show sessions 1 … 10, in `eon session ls` order (`[session] number_keys`) |
| Ctrl+Tab / Ctrl+Shift+Tab | Select the next or previous tab, wrapping around |
| Alt+H / Alt+L | Select the column to the left or right, continuing into the previous or next tab |
| Alt+K / Alt+J | Select the previous or next pane in the column |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the selected pane into the column to the left or right; at the tab's edge a stacked pane opens a new column |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane up or down in its column |
| Ctrl+Alt+Shift+H / Ctrl+Alt+Shift+L | Move the active tab |
| Alt+Shift+T | Create a tab and choose its directory in quick search |
| Alt+M | Create a pane in the selected column |
| Ctrl+Alt+M | Create a column right of the selected one, up to four per tab |
| Alt+Shift+Z | Show the selected pane alone across the tab, or restore the columns; changing the selected pane, tab or layout also restores them |
| Alt+W | Close the selected pane, stopping its terminal; the final tab's last pane quits the window, or closes the session while others remain |
| Alt+Shift+W | Close the active tab; closing the final tab quits the window, or closes the session while others remain |
| Alt+Shift+N | Open an independent Eon window (also the desktop launcher's **New Eon Window** action) |
| Alt+Shift+D | Detach: hide the window, keep terminals running |
| Alt+Shift+Q | Quit this window, stopping its terminals; asks first while one runs something. On macOS, Command+Q does the same |
| Alt+Z | Show the tab's quick search, or hide it when quick search is shown |
| Alt+Shift+J | Open or hide the tab's Git popup |
| Alt+Shift+L | Open or hide the tab's Agent popup |
| Alt+Shift+O | Open or hide the tab's Editor popup |
| Alt+Shift+Y | Show the tab's Files, or hide it when Files is shown |
| Alt+Shift+A | Open or dismiss the tab's Anima popup |
| Alt+Shift+M | Open or dismiss the tab's Commands popup |
| Alt+Shift+E | Open the shown terminal's scrollback in the configured editor, or dismiss it |
| Alt+Shift+S | Open the Sessions picker; in the picker, Alt+Shift+S or Enter shows the highlighted session, Enter on an unmatched name creates one, Ctrl+X closes the highlighted one |

### Eon Bar and tabs

The top row is the native Eon Bar. Its `+`, `?`, and `×` controls create
a tab, open Shortcuts, and close the active tab. The empty space
between the scrollable tabs and those controls moves the undecorated window;
tabs, controls, pane headers, and terminal content remain ordinary targets.
A zoomed tab shows `▣` after its name, the same Restore mark as the pane
header button. Its accessible name says `, zoomed`.
While the window holds two or more sessions, or its one session has a name,
an outlined label before the tabs names the shown session, or gives its ID
such as `s2`, followed, when there are several, by its position and the
count, as in `notes 2/3`.
Clicking it, or Enter after Tab focuses it, opens the Sessions picker as
Alt+Shift+S does, and its tooltip names that key. With the Sessions popup
turned off the label is plain text. While another session has an agent that
needs input, a ◆ badge sits on the label's corner, also while the sidebar is
shown, and its accessible name adds `, another session needs input`. A long
name is shortened while the position stays whole; quota chips hide before the
label, and the label hides before the tabs would lose their room.
Pinned CodexBar CLI supplies read-only Codex and Claude quota from their local
authentication. Codex also needs a compatible `codex` executable on `PATH`.
Codex chips show its white mark and Claude chips its orange splat.
Both providers show one chip per available window, five-hour then seven-day,
then one Claude chip for each model-scoped weekly window, such as `Fable`.
Each window shows its remaining percentage; hover lists when it resets, in
local time, the pace, and whether the reading is old. Missing resets stay unknown.
Chip text stays neutral. A thin bottom meter shows the remaining percentage,
coloured by pace; old and reset-less readings use a neutral meter.
Readings say `old` once they are more than an hour old and disappear after 24
hours. Chips hide one by one under width pressure, the one closest to the tabs
first, before tabs and controls, and unavailable quota leaves the bar unchanged.
See [quota behavior](CONFIGURATION.md#agent-quota).
After the quota chips come three readings, each an icon and a percentage such
as `12%`: a processor chip shows whole-machine CPU use over the last 2 seconds,
a memory stick whole-machine memory in use, and a drive how full the filesystem
holding your home directory is, refreshed every 30 seconds; hover them for
their names and details, such as the disk's size, mount point and free space.
Each hover is a short list, one fact per line, that leaves out what the chip
already shows; screen readers read one sentence with every fact.
Their bottom meters show the percentage in use and turn yellow from 60%,
orange from 75% and red from 90% use by default, at
the levels [`[bar]`](CONFIGURATION.md#configuration-behavior) sets, which also
chooses the path the disk chip measures.
A reading that can't be taken shows no label rather than 0%. A 24-hour clock
showing local time sits at the left end of the bottom hints bar, as plain text
in the version label's softened cyan, and hides with it; hover it for the date
and UTC offset, and it stays hidden if the local time zone can't be
determined. The version label (`EON α0.1.24` for alpha, `EON β0.2.0`
for beta, or `EON 1.0.0` for stable), in softened cyan, sits at the right end
of the bottom hints bar and hides with it. It names the running supervisor,
so updating the installation doesn't change it. Older supervisors without
stage metadata retain an unmarked version label when Venus reloads. Hover
and screen readers spell out a supplied stage, such as `Eon alpha 0.1.24`;
hover opens above the label. The Eon logo leads the bar, before the tabs.
`[bar] widgets` chooses which of these show and in what order (see
[Configuration](CONFIGURATION.md#configuration-behavior)). When the bar runs
short of space the Eon logo hides first; then disk, RAM and CPU, before any
quota chip. The logo also hides whenever the
tabs would not all fit beside it, however wide the window, so tabs never lose
room to it, and it leaves room for a chip for each window each provider's latest
read reported and for each reading, all at their widest, so a changing quota,
reading, or a quota window's reset, never shows or hides it. It can
show or hide once, shifting the tabs, when a provider's quota first appears or
goes away (as it does when all its windows reset before a read), or when a read
finds the provider has gained or lost a window.
See [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar) for the details of each
item.

The bottom hints bar starts visible. Alt+Shift+B hides or restores it for this
window, returning or reserving its space without restarting terminals; the
choice lasts through tab and popup switches and resizes. Each press toggles
once; repeats and releases never reach the terminal, and toggling keeps focus
and the selected tab and pane. Hidden hints leave no accessible footer; visible
hints are read-only, with no extra focus stops or announcements. The
initial window adds the footer's space, so configured `rows` stay terminal
rows. Hints follow the current bindings, enabled popup catalog and native
focus. Each set of modifiers shows as one keycap, named in full: `Alt`,
`Ctrl+Alt`, `Alt+Shift` and `Ctrl+Alt+Shift` (on macOS, `Option`,
`Control+Option` and so on). The keycaps form one cluster centred on the
window. Keys without modifiers follow them, written out: `Escape terminal`, and
the arrows while the tabs, panes or agent sidebar have focus; while the
attention inbox is open, the cluster is only the inbox's own keys. Pointing at
a keycap opens a popup above it with its keys in two sections side by side:
Actions, by what they act on (tab, pane, window, then the hints and sidebar
toggles), and Popups, with custom popups after the built-in ones. Each key has
a short label such as `New tab` or `Detach`; the keycap below already names the
modifiers. The popup stays open while the pointer is over it, and resting the
pointer on an action shows what it does as full help says it, in a box just
above. A section taller than the window has room for continues in the next
column; when the columns are wider than the window, the last one that fits ends
with `Alt+/ lists all`. The popup never takes focus. Escape or a click closes it
until the pointer leaves the keycap and the popup; Escape still does what it
always does, a click on a keycap does nothing else, and a click over the popup
goes where it would without it. Narrow windows give way in
this order: the cluster moves to stay one padding clear of the clock and the
version label, then the version label hides, then the clock hides, then keys
without modifiers drop whole (`Escape terminal` last), and in the narrowest
windows the keycaps switch to the short legends `A`, `CA`, `AS` and `CAS`;
keycaps never drop. With Eon's packaged font (DejaVu Sans) at the default size,
a 1280-pixel-wide window shows every hint centred, clear of the clock and the
version label; with the terminal focused, an 800-pixel window still shows
both, a 400-pixel window the full-name keycaps alone, and a 330-pixel window
the short legends. Screen readers read each
keycap by its full name, such as `Alt+Shift shortcuts`, with its sections and
every chord in them and what it does as full help says it. Alt+/, the Alt+Q
inbox, Alt+Shift+Q quit, Alt+G and Alt+Shift+G, F6, Ctrl+Tab and Ctrl+Shift+Tab, and Quick search (Alt+Z) have no
hint but stay in full help. Each pane is its own
rounded card, stacked without gaps in its column's allocated card area. Even
outer gaps surround and separate the allocated column areas. Pane titles sit
in rounded edges, with neutral outlines sized by `border_width`. Collapsed rows'
corners turn toward the expanded pane, and their sides join the next card in
that direction.
Pane labels come from Runtime: the pane ID, then a pinned name, otherwise its
sanitized terminal title, otherwise its compact working directory. A differing
terminal directory adds ` · DIRECTORY`, using a relative or shorter compact path.
Labels keep explicit connecting, unavailable and offline states. The ID is dimmer
when the shared background and opacity permit readable contrast; narrow labels
keep the ID when it fits, while accessibility and close listings retain the full
bounded label. Metadata and names are display text, never command input.

The selected pane has a brighter label and a whole outline in
its focus colour, at the configured focus border width. Hover fills a rounded
title pill, and F6 keyboard focus colours the title's span of the header edge mint. Compact-gap
and frameless titles retain their hover and focus pills. Turning pane frames off leaves labels and
their interaction feedback. The hints sit bare in the margin below the cards. Hiding the footer returns its row to terminal content.

With pane frames on, each column has one continuous terminal-background fill,
from its Stack tab's contour down to the column's rounded bottom.
The Stack band, stacked title rows and the space between cards share the
expanded terminal's background colour and opacity. Pane header bands, including
lone and zoomed panes, keep that shared fill; terminal edge colours never extend
into header chrome. Outside the shared title contour and outer corners, the gaps and
footer stay the opaque bar colour. The Agents fill follows the same contour.
Header scrolling keeps the fill's top fixed. Frameless mode keeps terminal
background strips across the window below the bar, including the sidebar and
footer.

Each column card's top line carries a Stack tab, labelled `Stack` when the tab
has one column and `Stack 1`, `Stack 2`, ... from left to right when it has
several; Stack N holds the panes whose `column` is N in `eon session --json`.
The Stack band is one title row plus the tab's rise, a third of the title
padding (24 px at the default font). A first collapsed pane's title row starts
that far below the band's top (14 px below the Stack line at the default
font). The expanded pane's framed header, a lone and a zoomed pane's included,
has one continuous raised band: the edge runs straight over the fitted title,
empty middle and right action group between rounded outer corners. A collapsed
pane shows its title and any visible buttons in gaps in its row's line. The
last collapsed title sits in the column's bottom edge. When room permits, the
expanded band starts one collapsed row's height below the line of the
collapsed title above it, or below the Stack line when the first pane is
expanded (20 px at the default font), including a lone or zoomed pane. For the
first expanded pane, the space needed for that top gap precedes its header and
the remaining reserve follows its terminal. A column's terminal keeps one
height whichever pane is expanded. Tight text or stroke clearance leaves the
expanded title in a compact gap. The column's sides run from the Stack line
down to the first pane's rounded top corners in the neutral border colour and
width, even while that pane is selected. Headers scroll below the band. On a
window too short for the band, a first header and one padded terminal row, the
band yields whole and the label goes.

Each pane header has a right action group: Zoom (`□`, Restore `▣` while zoomed)
and Close (`×`). The expanded pane's framed header keeps both in its band and a
collapsed row keeps them in a gap in its line; frameless headers keep the
controls inline. The title and scrollback indicator fit to its
left. Both controls disappear together if two full targets cannot fit; tight
clearances retain a compact gap. Stack, Agents, popup and
dialog headings have no pane actions, and a popup covers the underlying headers.
Popup and Agents headings have a Hide button of their own, and each Stack tab
a Close button; both are described below.
Hover and press highlight the individual button. A one-pane tab keeps a dimmed,
disabled Zoom button in its slot.

F6 cycles to pane chrome. Tab and Shift+Tab cycle the selected pane's title, Zoom
and Close, skipping disabled Zoom, then the selected Stack's Close while it
shows; Enter or Space activates a button. Up/Down
select another pane and return focus to its title. Action focus uses a transient
accent pill outline. Zoom first selects the named pane if necessary and then uses
Alt+Shift+Z's local view behavior; another action or zoom toggle cancels a pending
selection. Close targets its own pane exactly as Alt+W does, with the same
final-pane confirmation. After a keyboard Close, focus returns to the selected
pane's title. Escape cancels that prompt; Enter closes its held target.
Agents can use `eon pane close PANE`; zoom is window-local view state.

Panel headings default to `"connected"`, a straight rounded top enclosing the
title and actions. Popup and Agents headings have one Hide button (`–`) at the
right, with the name to its left. With `[panels] heading = "notch"`, the button
sits in a second notch mirroring the title's. The popup's
button hides the shown popup exactly as `eon popup hide POPUP` does, and the
Agents button hides the sidebar as Alt+Shift+H and `eon sidebar hide` do. Hover
fills the button's pill, a held press turns it accent, and releasing over it
activates it; a press held while another popup takes the heading does nothing.
With pane frames off, the popup keeps its selected heading outline, and the
Agents button shows beside its plain label with no notch or outline. With
frames on, a heading without clearance for its tab keeps a compact title and
the button sits in its own gap in the top edge. A heading too narrow for a
full button shows none. With keyboard focus in the sidebar, Tab and Shift+Tab
move between its rows and Hide, Enter or Space activates it, and Up or Down
returns to the rows. F6 skips a sidebar with no rows, so Hide has no keyboard
stop there and Alt+Shift+H hides it. The popup's button has no keyboard stop.
Most popups still hide on their own shortcut; Sessions' key confirms its
selection instead, and quick search and Files switch to each other. Screen
readers list `Hide popup <name>` and `Hide agent sidebar` as buttons.

Each Stack tab has one Close button (`×`), in the same right notch when
`heading = "notch"`; the label fits to its left. Every Stack shows it unless a
pane is zoomed or the tab is too narrow for a full button; with pane frames off
it sits beside the plain label with no notch or outline. Close always asks
first, whatever the Stack runs, even for one idle pane, and whatever
`warning_close_last_tab` is.
The prompt lists every terminal the close would stop, one row each, marks the
ones still running with `· running`, and for the last Stack of the last tab
says that it closes the session or quits the window. Esc keeps the Stack.
Enter closes exactly the listed terminals. It is ignored for the first moment
(400 ms) after the list appears, so a key pressed before the list could be
read never confirms it; press Enter again. If the Stack changed while the
prompt was open (a pane joined or left it, or the close became a final one),
Enter stops nothing and the prompt shows again from the new list, starting
"This Stack changed.". Tab reaches the selected Stack's Close after the
pane's own buttons; Enter or Space activates it and Up or Down returns to the
pane title. After a confirmed close, keyboard focus returns to the pane
title; Esc leaves it where it was. Screen readers list `Close Stack` or
`Close Stack N` as a button on every visible Stack and can focus any of them.

The expanded pane's header uses the continuous band above; collapsed pane
titles sit in their row's line. Stack tabs, popups, Agents and
native titled dialogs share a fitted tab style. Their outline rises seamlessly
from the left border over the fitted name,
then returns into the top edge with a tangent S curve spanning 1.5 times its
rise. The curve begins beside the visible text, including after truncation.
Stack, popup and Agents tab tops sit one card gap (8 px) below the bar's pills.
Popup, Agents and dialog tabs add no height or terminal rows inside their cards.
Each pane header uses the shared title row height. A Stack with several panes
reserves half a title row plus one tab rise (14 px at the default font) from
the terminal. A lone or zoomed pane reserves only half a title row (10 px at
the default font), costing about one terminal row. Multi-pane terminal
capacity is unchanged by the first-pane gap.
The reservation yields before the last padded terminal row on short windows.
A tab keeps its stroke clear of text and neighbouring chrome; where that
clearance cannot fit, the title keeps a compact gap in the top frame line.
A title is omitted only when it cannot fit. A tab keeps full accessibility
names and adds no input targets other than the Hide and Stack Close buttons
above; decorative
pane caps outside a header row do
not extend that row's click or accessibility bounds. The Stack label is a visual
heading only. Keyboard title focus colours the title's span of the continuous
pane edge; hover fills a rounded title pill. Button focus uses its accent pill
outline. Compact and frameless titles retain their focus pill.
Popup tabs keep their outline with pane frames off; Stack and Agents labels show
plain then, and the Stack band stays reserved. On a light terminal background a
Stack label darkens only as far as 4.5:1 contrast needs. Dialog tabs stay inside
their existing dismissal bounds and use the dialog's opaque background; other
tabs inherit their terminal background colour and opacity.

Keycaps keep their dark fill and brighten under the pointer; their legends,
popup keys and expanded hint keys use grey text roles. Mint marks keyboard
focus and pressed controls. Selected tabs use a brighter fill; the current
Agents row has a one-pixel outline in the configured focus colour, with no
selection fill. Hover or keyboard focus adds a dark fill to every row while
the current outline stays visible. Keyboard focus adds a mint outline one
pixel inside the current outline. Resting tab numbers and sidebar heading
numbers are dimmer than their names. The whole selected tab label brightens.
The compact tab range is 1..9; Alt+0 still selects the tenth tab.
Copy/Paste and Open link stay in full help, without footer hints. The `Alt`
popup lists one `focus` row, HJKL · 1..9, the `Ctrl+Alt` popup one row for the
HJKL pane moves, and the `Ctrl+Alt+Shift` popup the HL tab moves. Full help and
screen-reader labels keep complete chords and action names. The header `?`
always shows; it and Alt+/ open the complete Shortcuts viewer, including the
hints toggle. Eon has no minor modes yet.

Tabs own launch directories; changing a shell's directory does not rename or
retarget its tab. Using a folder from quick search or Files changes the
directory used by future terminals in the tab without moving existing
terminals. Git and Agent are tab-scoped
terminals whose terminal state survives hiding and reopening. When the tab's
directory changes, a running Agent moves out of its popup into a pane of the
tab, with its conversation and old directory intact, and the next Alt+Shift+L
starts a fresh agent in the new directory. Git restarts in the new directory
the next time you open it. Anima is transient:
closing its popup stops playback. Closing a transient popup such as Commands,
Sessions or Anima, including after a session switch, returns to the popup
it covered, such as Agent, or to the pane when it covered none.

Editor starts the configured terminal editor in the tab's current directory.
Hiding Editor or changing the tab directory keeps the editor and its open
buffers. After the editor exits, Alt+Shift+O starts a fresh instance in the
tab's current directory.

Each tab has one Files popup, Yazi at the tab's directory, with two screens:
quick search over recent folders, and Files itself. Alt+Z shows quick search
and Alt+Shift+Y shows Files; each key hides the popup when its own screen is
already shown. In quick search, Enter uses the highlighted folder for the tab,
Tab moves Yazi there without using it and shows Files, and Esc or Ctrl+C
closes the popup. In Files, Alt+Enter uses the hovered folder, or Yazi's
current one, for the tab without moving Yazi; Tab returns to quick search
without moving Yazi; Enter on a folder moves into it; Shift+Tab shows the
hovered file's details. Esc first leaves a visual mode, selection, filter,
find or search, one per press, and otherwise closes the popup; Ctrl+C does the
same and no longer closes a Yazi tab. In a Files left over from a supervisor
recovery, they quit Yazi instead. Every other Yazi key, including file
operations, works as usual; Z, q and Q do nothing. Files keeps its place when
hidden and when the tab's directory changes. Only a folder you use enters
Zoxide history, as Zoxide records it (by default it skips your home
directory); browsing and searching add none, and `[shell] zoxide = false`
stops even that. Enter on a text file opens it in a new pane running your editor
([`[editor] command`](CONFIGURATION.md#editor): Helix by default, or `$VISUAL`
or `$EDITOR` in the package without Helix) beside your
work and hides Files, one pane per selected file; other files keep Yazi's
default openers. A new tab must have its directory first: until then, Enter on
a text file refuses with `choose-directory-first`.

A new tab has no terminal until you choose its directory: it starts on quick
search. Using a folder starts the tab's first terminal there and hides the
popup; closing the popup starts it at the tab's starting directory. Either
way, the same Yazi stays as the tab's Files.

Commands lists Eon commands you can search and run: new tab, pane or
window, close or move the tab, quit this window, switch, create, rename or
close a session, show the session, windows, versions or config
path, stop old-version windows, check for updates, install an update, and
command help. Each line shows the `eon`
command it runs. Enter runs it from the popup
for that tab, with the command's own checks and confirmations. New pane
closes the popup first, as Alt+M does, and the pane joins the tab's selected
column. Other actions close the popup when they succeed, except a close from
the session list, which lists again; listings, stopping old-version windows,
update results and failures stay until you press Enter. Esc closes it
without running anything. Stop old-version windows stops every window that
another build started, so in a window of an older build it also stops the
windows of newer ones.

Search `update` for **Check for updates** (`eon update --check`) and
**Install update** (`eon update --install`). Install displays the target and
source and asks its own `[y/N]` question in the popup's terminal. Both actions
use the same configured target or safe discovery and refusal guidance as the
CLI (see [Update](#update)). The Window menu's **Updates** entry opens Commands;
search `update` there. It is available when Commands is enabled. Installation
affects fresh launches through the reported launcher; running windows keep
their builds and terminals.

Sessions (Alt+Shift+S, or Commands > Switch session) lists this window's
sessions most recently shown first, each by name or ID with its tab count and
` · needs input` when one of its agents needs input, the shown one last and
marked, so Alt+Shift+S then Enter returns to the previous session. In the
picker Alt+Shift+S works as Enter, so pressing it twice, like a double
Alt+Tab, switches back and forth between two sessions. Alt+Shift+1 through
Alt+Shift+9 show a session by its position in the window's list, the one the
Eon Bar label counts (`notes 2/3`), and Alt+Shift+0 the tenth. Type to filter by
name. Enter shows the highlighted session, and Enter on a name that
matches no session creates a session with that name and shows it. A name
that is part of an existing one matches it, so create that session with
Commands > New session and then rename it. Ctrl+X closes the highlighted
session after asking, as Close this session does, and then lists the rest
again; closing the picker's own session closes the picker too. Esc or Ctrl+C
closes the picker without changing anything more. Rename
session names the popup's session (an empty name clears it), and Close this
session asks before it stops every terminal in it.

### Agent sidebar

In full Eon windows the agent sidebar lists, in tab order, the tabs whose
coding agents report their status. Its card looks like a pane card, with the
same outline, background and background opacity. Each tab gets a heading such
as `3  site`, then one row per reporting terminal: a popup's row first, then
the panes' rows in pane order. A row shows:

- the state: ◆ needs input, ✕ error, ✓ finished, stepping braille dots while
  working (still without GPU rendering or with
  `[session] animate_working_badge = false`), three still dots in a row while
  the agent waits on background work (it reported done or idle while processes
  it started still run), or ○ idle; the dots take the window's focus colour
  (`[terminal] focus_color`);
- the Claude or Codex icon;
- a Claude Code session's name, such as `eon-9a` for a session started
  with `claude --name eon-9a`; otherwise the title the agent reports, and
  without that, the app's name, such as `Codex`;
- how long ago the state changed (`now`, `4m`, `2h`, `3d`);
- the pane (`p3`) or popup it runs in.

On Linux, the four measured Codex code-mode/ChatGPT REPL helpers alone leave
the row finished or idle. Builds, servers and watchers started as separate
processes keep the dots, including those started by a helper. Other MCP
servers or installation layouts, replaced helper executables and Darwin keep
the raw background behavior. Work inside an exempt REPL itself, without a
child process, cannot be distinguished by this observation.

A needs-input row adds the agent's question. `+2` counts the terminal's other
status records. A dimmed row marked `status stale` has not reported for more
than 3 seconds. An Agent popup whose agent does not report shows `status
unavailable`. An agent running in a popup has a window mark larger than the
label text at the row's right edge, whether or not the popup is shown. Text
stops before the mark, and the row uses the same fill as a pane agent. The
attention inbox uses the same treatment for agents in the shown session. The
current terminal's row has a one-pixel outline in the configured focus colour,
with no selection fill. Hover or keyboard focus adds the dark fill to any row;
mint keyboard focus sits one pixel inside the current outline. While no agent
reports, the sidebar says so.

Click a row, or press Enter or Space on it, to jump to that agent: Eon selects
its tab and shows its pane or popup, and keyboard focus moves to that terminal.
If a transient popup covers the agent, the jump
only selects the tab, and focus moves to the tabs. F6 moves keyboard focus from
the terminal into the sidebar, back to the row it last focused while that row
remains, then to the tabs and panes. Up and Down move between rows, and Escape
returns to the terminal. Tab moves from the rows to the heading's Hide button
and back; Enter or Space on it hides the sidebar, as clicking it, Alt+Shift+H
and `eon sidebar hide` do. The wheel and touchpad scroll overflowing rows when
pointed anywhere over the sidebar card. A slim neutral thumb beside the rows
shows the viewport's position while rows overflow. Keyboard focus still scrolls
its row into view; pointer scrolling keeps keyboard focus and the active
terminal in place.

Alt+G jumps to the next agent that needs attention, in any session of the
window, whether the sidebar is shown, hidden or turned off; jumping to an agent
in another session shows that session. A finish or error stays new while
another app has focus, and while its session is not shown. It
stops being new when the window has focus with that agent's terminal selected;
a finish also stops being new once that terminal reports work again.
Agents that need input come first, then errors, then finished agents,
each oldest first; an error or a finish leaves the walk once you have looked at
that agent. Each press moves one agent further, wrapping around; Alt+Shift+G
steps back. The walk starts again from the front
once you select another terminal some other way, such as with a click, a
sidebar row or Alt+H, or when an agent needs something more urgent than the one
you last jumped to, for example input while you were walking finished agents.
A jump to an agent under a
transient popup only selects its tab. With no
agent waiting, the keys show "No agent needs attention" and move nothing.

### Attention inbox

Alt+Q opens the attention inbox under every sidebar setting. It lists agents
needing input, in error or finished, in the same order as Alt+G. The All,
Needs input, Error and Finished filters show counts over the whole queue.
Left/Right cycle the filters; Up/Down highlight an agent. Enter, Space or a
click jumps to that agent and closes the inbox. Escape or Alt+Q closes it
without a jump. The wheel, PageUp/PageDown, Home and End scroll the list.

Rows in the shown session name the tab and pane or popup. Rows in another
session show its name, or its ID when unnamed; jumping to one shows that
session. Opening the inbox does not mark any agent's attention as seen.
Agents read the same queue as `attention_queue` in `eon session --json`.
A popup configured on Alt+Q takes the shortcut.

### Scrolling and selection

When you scroll above live output, `↓ N rows` shows how many display rows back
you are. It appears in Eon's pane header or at the top right of eonbase, updates
as output continues, and disappears at live output. Click the whole pill to
return there.

Two clicks select a word, three select a
space/tab-delimited span, and four or more select the logical line across soft
wraps. Dragging extends that granularity; a delayed single click returns to
cell selection. Hold a drag in or above the pane's top row to keep extending
the selection into history. Release copies to both Wayland clipboard destinations, and
Ctrl+Shift+C copies the frozen selection even after output changes.

## CLI commands

`eon -h` and `eon --help` show grouped commands,
short descriptions, and examples without starting a session. After a command
family or command, as in `eon pane --help` or `eon pane send -h`, they show
only that family's or command's lines. Help uses color
on terminals; redirected output is plain text. A nonempty `NO_COLOR` or
`TERM=dumb` disables color. Invalid syntax points to `eon --help`, whose last
line points agents to `eon --skill`.

| Command | Result |
|---|---|
| `eon` | Present the current window or start its first tab on quick search |
| `eon [--dir DIR] [--no-anima]` | Start the window with its first tab's terminal in DIR, with no quick search and no startup animation, or start it on quick search without the animation. `--dir` fails (exit 1) when the window already runs; use `eon tab create --dir DIR` or `eon window new --dir DIR` then |
| `eon anima [STYLE] [CHILD OPTIONS...]` | Run pinned Anima in the caller's terminal without starting a session; `--help` lists styles and options |
| `eon run [--dir DIR] [--no-anima] [-- COMMAND...]` | Start a fresh window on quick search, optionally for an exact first command; `--dir` and `--no-anima` as for `eon` |
| `eon window new [--dir DIR] [--no-anima]` | Open an independent Eon window and print its window ID; `--dir` and `--no-anima` as for `eon` |
| `eon window ls [--json]` | List every running window, the default window first: its ID (shortest unique prefix in text), name, Eon version, start time, live terminals and whether this `eon` controls it directly. Each window this `eon` cannot stop gets a line saying why. JSON adds the full `id`, `current`, `generation`, `state`, `busy`, `attach`, `stop` and `executable` |
| `eon window attach [ID]` | Present a window after detach (no ID: `default`), through the Eon build that started it when needed |
| `eon window stop ID [--json]` | Stop only the selected window's terminals; `--json` prints `[{"window": ID, "results": [...]}]` |
| `eon window stop previous [--json]` | Attempt Stop in every running window that another Eon build started, in either namespace, the caller's window last |
| `eon window stop all [--json]` | Attempt Stop in every running window, the default one included, the caller's window last |
| `eon quit [--json]` | Quit the window of the calling terminal, stopping its terminals; it asks first only while another terminal of the window runs something other than its idle shell, and names those terminals (the Files and Git popups never count; the Agent popup does); outside Eon, quit the only running window, or list the running windows and stop nothing when several run |
| `eon quit --all [--json]` | Attempt Stop in every running window, as `eon window stop all` does, asking for each window as `eon quit` does |
| `eon session ls [--json]` | List the window's sessions in order, with each one's local ID, persistent ID (JSON), name, whether it is shown, its tab count, its most urgent agent attention state, and its recency (0 for the shown one, then 1 for the one shown before it, and so on, the Sessions picker's order) |
| `eon session [--json]` | Inspect the live tab, pane, popup, and terminal mapping, with each terminal's agent attention state, app and name; JSON includes each terminal's full agent attention record and the Codex and Claude quota readings, and the calling terminal in `caller` (null without `EON_TERMINAL`; a `caller` line in human output only when present); JSON `running` holds this window and the other running Eon windows, each with its ID, generation, name, start time and live terminal count; JSON `busy` lists the window's busy terminals, each with its session, tab, pane or popup, terminal and name, and `omitted` counts those past 32, with one `busy` line per terminal in human output |
| `eon session registry [ID] [--json]` | Read host registrations without a window; registration alone says nothing about running terminals |
| `eon session --id ID [--json]` | Inspect that exact member without changing the shown session |
| `eon session new [--name NAME] [--no-focus] [--json]` | Add a session after the shown one, with one tab on quick search, and show it; with `--no-focus` the shown session stays. The reply names the new session's ID in `created` and its persistent ID in `created_persistent_id` (also printed in human output) |
| `eon session switch SESSION [--json]` | Show session SESSION, with its tabs, panes and popups as it left them |
| `eon session rename [SESSION] NAME [--json]` | Name session SESSION, or the caller's session; an empty NAME clears the name |
| `eon session close SESSION [--json]` | Stop every terminal of SESSION and remove it, asking first without `--json`; closing the last session quits the window |
| `eon tab create [--json]` | Create a distinct pending tab on quick search, even while another tab waits for its directory. The JSON reply names the new tab in `created` as `{"tab": "tN", "pane": null}`: it has no pane until its directory is picked, and the pick's reply (`tab directory`, or `popup hide` of its quick search) does not name that pane |
| `eon tab create [--json] [--no-focus] --dir DIR [-- COMMAND [ARGS...]]` | Create a tab at DIR without quick search, optionally running an exact command from a calling terminal; `--no-focus` preserves the active tab, selection and popup. The JSON reply names the new tab and its pane in `created`, as `{"tab": "tN", "pane": "pN"}` |
| `eon tab close TAB [--yes] [--json]` | Close any tab of the addressed session without changing the active tab, selection or shown popup; busy closes ask first, as described after this table. The final tab quits the window, or closes the session while others remain, asking first from a terminal |
| `eon tab directory TAB [--json] -- DIRECTORY` | Set a tab's directory for future terminals; on a pending tab, use DIRECTORY as quick search would: start its first terminal there and hide its Files |
| `eon tab switch TAB [--json]` | Make tab TAB the active tab of the session, as clicking it does; from an Eon terminal in a hidden session, its active tab changes and it stays hidden. Desktop focus stays where it is. A TAB that is not live exits 3 (`unknown-tab`) |
| `eon tab move left\|right [--json]` | Move the active tab, or the calling terminal's tab, one position |
| `eon pane create [--json] [--no-focus]` | Create a pane in the active tab's selected column, or in the calling pane's column; with `--no-focus`, the tab's selected pane and any shown popup stay as they are. The JSON reply names the new pane and its tab in `created`, as `{"tab": "tN", "pane": "pN"}` |
| `eon pane create [--json] [--no-focus] -- COMMAND...` | From an Eon terminal, run COMMAND exactly as given (no shell) in a new pane of the caller's column, at the tab's directory, with `--no-focus` and `created` as above; refused with exit 1 outside an Eon terminal |
| `eon edit [--json] -- FILE` | Open FILE in the configured editor: packaged Helix uses the tab's Editor popup by default, retaining its buffers and unsaved edits; `[editor] open_in = "pane"`, other editors, directories, disabled Editor, an existing Editor without Eon's listener, and no-Helix builds use a new pane placed as `pane create` places it; JSON names Editor in the tab's `selected_popup`, or the new pane in `created` |
| `eon pane close PANE [--yes] [--json]` | Close any pane of the addressed session, stopping its terminal without changing the active tab, selection or shown popup; busy closes ask first, as described after this table. The final tab's last pane quits the window, or closes the session while others remain, asking first from a terminal |
| `eon column close PANE --yes [--json]` | Close the column (Stack) holding PANE. Without `--yes`, list every terminal it would stop and fail `not-confirmed` (exit 1); never reads stdin. A changed scope fails `close-changed` (exit 1) and stops nothing |
| `eon pane move up\|down [--json]` | Move the selected pane, or the calling pane, one position within its column |
| `eon pane move left\|right [--json]` | Move the selected pane, or the calling pane, into the neighbouring column, as Ctrl+Alt+H/L does |
| `eon pane read TARGET [--json]` | Print the visible text of pane or popup TARGET, in any session of the window, without changing anything; see [Reading a terminal](#reading-a-terminal) |
| `eon pane history TARGET [--json]` | Print pane or popup TARGET's retained history followed by its visible text, in any session of the window, without changing anything; see [Reading a terminal](#reading-a-terminal) |
| `eon pane history TARGET --edit [--json]` | Open that text in the configured editor in a Scrollback popup over TARGET's tab, as Alt+Shift+E does, when the tab is the shown session's active tab; see [Reading a terminal](#reading-a-terminal) |
| `eon pane send TARGET [--enter] [--force] [--stdin] [--json] [-- TEXT]` | Paste TEXT, or standard input with `--stdin`, into pane or popup TARGET in any session of the window, then with `--enter` press Enter; see [Sending to a terminal](#sending-to-a-terminal) |
| `eon popup hide POPUP [--json]` | Hide popup `uN` of the session as its window's dismissal would: a keep-alive popup keeps running, a transient one stops, and a pending tab's Files starts the tab's first terminal at its starting directory |
| `eon sidebar show\|hide\|toggle [--json]` | Show, hide or toggle the window's agent sidebar, as Alt+Shift+H and the sidebar's Hide button do, and print the session, whose `sidebar` reads `open`, `closed` or `off`. `show` and `hide` set the state directly in Runtime, so repeating either changes nothing; the command or keypress that ran last determines visibility. With `[sidebar] enabled = false`, `hide` succeeds and `show` and `toggle` fail `unavailable` (exit 1) |
| `eon focus PANE\|left\|right\|up\|down [--json]` | Focus pane PANE, switching to its tab, or traverse the session; switch tabs with `eon tab switch` |
| `eon versions` | Print Eon, protocol, and component identities |
| `eon report` | Print a Markdown bug report for this window, with paths under your home as `~` and long environment values (8 bytes or more) removed, to read and then paste into the bug form; see [Troubleshooting and reporting](#troubleshooting-and-reporting) |
| `eon update --install [--yes] [--json] [--profile ABS --entry NAME \| --prefix ABS]` | Update one eligible Nix entry on its original ref or one managed Linux prefix through its bundled installer for fresh launches; JSON/nonterminal consent requires an explicit or configured target; see [Update](#update) |
| `eon update --check [--json] [--profile ABS --entry NAME \| --prefix ABS]` | Report this installation's owner, source and installed build, whether an update exists, and whether Eon may install it, without changing anything; see [Update](#update) |
| `eon config path` | Create the configuration root and print the full path of its settings file, `config.toml`, which need not exist yet |
| `eon config dir` | Create and print the configuration root |
| `eon config defaults [--json]` | Print built-in defaults as commented TOML or a nested JSON object, without reading user configuration |
| `eon config schema --json` | Describe active configuration keys, types, defaults, bounds, choices and when changes take effect, without reading user configuration |
| `eon config check [--json]` | Check config.toml for settings Eon would reject, without starting Eon or changing state |
| `eon --skill` | Print the `eon` agent skill this build ships, without starting a session or reading configuration |
| `eon -V`, `eon --version` | Print this build's Eon version as `eon VERSION`, without starting a session or reading configuration |

Window IDs: `default` is the window bare `eon` opens. One that another Eon build
opened that way, such as the old build's window after an upgrade, has the 32 hex
digits of its generation as its ID, and an independent window has the ID
`eon window new` printed. `eon window attach` and `eon window stop` take a full
ID or any prefix of at least 4 characters, which must start exactly one running
window's ID. One that starts several, even when it is one window's whole ID,
exits 3 and lists them, and one that starts none exits 3 too. Text output shows
each ID as its shortest unique prefix; scripts should keep the full `id` from
`eon window ls --json`, since a short prefix can become ambiguous later.
`default` also answers to the hex digits of its generation, which the detach
notice and the log line after Eon Desktop exits print, since after an upgrade
`default` names the new build's window: `eon window stop 1a2b3c4d` stops that
window from its own build or a newer one.

A window holds one or more sessions and shows one at a time; hidden
sessions keep their terminals running. SESSION is a session ID (`s1`, `s2`, ...),
never reused by the window's supervisor, or a session's exact name. Names are
labels and need not be unique: a name that matches several sessions changes
nothing and exits 1 (`ambiguous-session`), naming their IDs.

Session commands inside Eon target the supervisor and session that own the
exact `EON_TERMINAL` caller, even when the invoked CLI is from another compatible
build. Outside Eon, they default to the invoked build's current generation in
the inherited runtime namespace. Human and `--json` output identify `generation`,
`window` (null in JSON for the default namespace), and the projected `session`.
A missing supervisor is not started implicitly.

Session commands accept `--window ID` before any `--` operand separator, for
example `eon session --window default --json` or
`eon pane create --window 3f9a --no-focus`. ID is a window ID as
`eon window ls` shows it, or any unique prefix of at least 4 characters:
`default`, the 32 hex digits of another build's default-namespace window, or
an independent window's eight hex digits. In an independent window, Eon
selects its current generation if live, otherwise its sole live generation, as
`window attach` does. The flag constrains a calling terminal: it never waives
its validation. Remove
`EON_TERMINAL` explicitly to act without caller context.
Eon calls Orbit-backed instances terminals in human output.
In JSON, `session` names a session (`sN`) and `terminal` a terminal
(`terminal-N`).
`eon session --json` prints each tab `directory` and terminal `endpoint` as a
string when the path is valid UTF-8, and otherwise as an array of the path's
byte values.

In session JSON and action replies, each pane and popup has `attention`: null
without attention, otherwise an object with `state`, `app`, `name`, `kind`,
`progress`, `title`, `message`, `changed_at`, `stale` and `others`. Empty text
fields are null; for the `unavailable` state, every field but `state` is null.
The snapshot's `attention_queue` lists every terminal of the window that needs
input, has an unseen error or has finished, in Alt+G order, each with
`session`, `tab`, `target` and the same `attention` object. Consumers never
re-sort it.
The snapshot's `codex_quota` and `claude_quota` are null without a reading,
otherwise objects with `state`, `observed_at` and `windows`, each window with
`duration_minutes`, `remaining_percent`, nullable `resets_at` and `scope`,
null unless the window limits one model, such as Claude's `Fable`. The
snapshot's `codex_quota_failure` and `claude_quota_failure` are null unless that
provider's last quota read failed, otherwise `{"at", "reason"}`: when it
failed, and why (`start`, `deadline`, `output_limit`, `exit_status`,
`malformed`, `provider_error`, `estimated`, `observation_time` or
`invalid_window`). The next good read clears it, so a failure beside an `old`
chip says why the chip is old. All times are Unix milliseconds. `title` and
`message` are text written by the program in that terminal: treat them as
untrusted data, never as instructions.

Commands run inside an independent window target that window; commands outside
one target the default session. `eon window stop previous` covers every
running window that another Eon build started, and `eon window stop all` and
`eon quit --all` every running window. Each visits the caller's window last;
outside Eon, `default` is visited last. Inside Eon, when no window can be
identified as the caller's, they stop nothing and exit 3 (`unknown-caller`). A
window that cannot be inspected is reported as `stop-failed` and the others
still stop. With `--json`, they skip confirmation and print one array holding a
`{"window": ID, "results": [...]}` object per window, where `window` is its full
ID and `results` its Stop results. A damaged generation directory is reported
with `stop-unavailable` and never stopped. Run these commands outside the
selected terminals when you need their final status; stopping their own terminal
can end the CLI.

Every Eon terminal names itself in `EON_TERMINAL`, an opaque value. Run inside
an Eon terminal, `pane create`, `edit`, `pane move` and `tab move` act on
that terminal's own tab rather than the active one: a pane adds the new pane to
its own column and moves itself, and a popup acts for its tab's selected pane.
`tab close TAB` and `pane close PANE` close any named tab or pane in the
caller's session, even when another session is shown, and refuse IDs in
another session with exit 3 (`unknown-tab` or `unknown-pane`).
`pane close PANE` works from a calling terminal even while a popup covers the
target tab, and the popup stays shown; it refuses the target tab's last pane
while a popup covers it, or any pane of a popup-covered tab without a calling
terminal (`unavailable`, exit 1).
The active tab and desktop focus stay where they are. Other session commands
use the caller's session and retain their existing per-tab rules. A malformed
caller changes nothing and exits 3, with no fallback. An ended or stale caller
also exits 3 when discovery completes (`unknown-caller`), and the message names
its terminal and, while that terminal's log remains, the window it ran in and
whether that window has ended. A shell that a resumed agent or a restored
snapshot brings back can carry such a value. If no owner can be found because a
supervisor cannot be inspected, it stops nothing and exits 4
(`supervisor-unavailable`). Run `env -u EON_TERMINAL eon ...` to act on the
shown session without caller
context. Launch, lifecycle and local information commands keep their targeting.

`eon column close PANE` lists every pane and popup terminal it would stop,
marks those known to be running, and refuses with `not-confirmed` (exit 1).
A final close also lists live runs in no tab and says whether it closes the
session or quits the window. This applies even to an idle or one-pane Stack,
independently of `warning_close_last_tab`. It reads no stdin, piped or at a terminal.
A Stack's Close button in the window asks under the same rule, and
`eon column close PANE --json` prints the list its prompt shows as data and
stops nothing.
Use `--yes` only when stopping all work in that invocation's listed runs is
authorized. Runtime binds consent to the generation, exact run IDs and the
close's consequence; if those change before the close runs, it stops nothing
and fails `close-changed` (exit 1). Busy marks only inform: work started later
in a listed run is inside the consent. `--json` refusals include the scope in
`close` beside `error`, with `generation`, `pane`, `end`, `session`, `terminals`
(each with its target, `run`, `name` and `busy`) and `untracked` runs.
Column targeting and popup rules follow pane close, except a caller may not
close the tab's only column while its popup is shown.

Before every `tab close` or `pane close`, the CLI inspects the session and
checks the named target, even with `--json` or `--yes`. When a terminal the
close would stop is busy, or Eon cannot tell, it asks once on standard error,
listing those terminals (`still running` or `busy state unknown`), and stops
nothing unless the answer is yes. At a terminal any other answer prints the
cancel note and exits 0; without one, only a piped `y`, `Y`, `yes` or `YES`
confirms, and anything else exits 1 with `not-confirmed`. `--json` never asks and never
reads standard input: it refuses a busy close with `not-confirmed` (exit 1)
unless `--yes` confirms that one close, for example
`eon tab close t3 --yes --json`. Busy terminals in other tabs do not count.
The check uses the latest inspection; Eon checks the target again before
stopping it, but not whether it became busy since.

A busy close of the final tab, or of its last pane, names what it ends, the
window or the session, whatever `warning_close_last_tab` says. An idle final
close asks only a person at a terminal, while that setting is `true` and
without `--yes`.

An eonbase window reports no busy terminals, so every tab or pane close there
counts as busy state unknown: a terminal asks, a pipe needs a yes, and `--json`
needs `--yes`.

An Eon terminal's PATH puts that terminal's own Eon build first as `eon`, so
`eon` keeps acting on its own window after you install a newer profile. That
includes `eon` and `eon window new` typed there, which open that same build;
open the newer build from the launcher or from a shell outside Eon. A session
command from another compatible build still finds the caller's supervisor in
the inherited namespace. Discovery validates private directories and endpoints,
has a bounded candidate count and timeout, and refuses ambiguous or incomplete
resolution. Without a caller, `EON_WINDOW_RUNTIME_DIR` selects an independent
window's current live generation or its sole live generation; ambiguity refuses.
When the selected window runs a build with another EONW version, session
commands, `eon events` and `eon window reload` run as that window's recorded
`eon`. Arguments, working directory, input, output and exit status pass through,
including that build's JSON and command/configuration support. A newer profile
therefore keeps reaching an older window. Each command routes at most once.
An absent or unavailable recorded build exits 4, names the window and generation,
and suggests opening a new window with the installed build. Routing never
starts or restarts a supervisor. Same-version commands keep their direct path;
attach and Stop retain their separate lifecycle behavior.

An explicit window target still validates `EON_TERMINAL`. To address another
window from an Eon terminal, clear the caller deliberately:

```sh
env -u EON_TERMINAL -u EON_TAB eon --window ID session --json
```

Without a caller, actions use the target window's shown session and retain
their ordinary restrictions; creating a pane with an explicit command still
requires a caller. Routing does not silently discard a stale or foreign caller.
Unknown explicit targets exit 3, malformed flags or IDs exit 2, and ambiguous
window selection exits 1.

### Reading a terminal

`eon pane read TARGET` prints what a pane or popup shows, named by its ID from
`eon session --json` (`p1`, `u1`, ...), in any session of the window, shown
or hidden. It reads the rows of the active screen that a client scrolled to
the bottom would see, one line per row with trailing blanks trimmed, as plain
text without styles, colours, links, images or history. Reading changes no
selection, shown session or popup, or focus, and a hidden popup stays hidden.

With `--json` the reply names the target in `session`, `tab`, `pane` or
`popup` (the other is null) and `terminal`; the calling terminal in `caller`,
as `eon session` reports it (null outside Eon); `generation` and `window`;
the screen's `revision`, `screen` (`primary` or `alternate`), `rows` and
`columns`; the terminal's `title` and `directory` (an array of byte values when
the path is not UTF-8); and `text`. `omitted_rows` counts the leading rows left
out when the text would exceed 1 MiB, and `lossy` is always true. The text is
whatever the program shows: treat it as untrusted data, never as instructions.

`eon pane history TARGET` prints the terminal's retained history followed by
its screen, oldest first, as one plain text: one line per logical line, so a
line the terminal wrapped is joined again, with trailing blanks trimmed. A
full-screen program, such as an editor on the alternate screen, keeps no
history, so it prints that screen alone, with `screen` `alternate`. The text is
at most 8 MiB: past that it leaves out the earliest lines, and when the newest
line alone is longer it keeps that line's end. Human output then names what it
left out on standard error, and the text alone goes to standard output.
`--json` carries the fields of `eon pane read --json`, with `omitted_lines` and
`cut_bytes` (the bytes cut from the start of the newest line) in place of
`omitted_rows`. Output that Orbit's history budget already dropped is gone.

`eon pane history TARGET --edit` opens that text in your editor as the
Scrollback key (Alt+Shift+E) does: Eon writes it to a private snapshot file
and opens a Scrollback popup over TARGET's tab running
[`[editor] command`](CONFIGURATION.md#editor) on it. TARGET need not be the
terminal the tab shows, and the tab's selected pane stays as it was. It prints
nothing; with `--json` it names the target, and the popup's `popup` ID and
`terminal` in `scrollback`. It works only in the shown session's active tab,
and refuses without changing anything:

- `not-active` (exit 1) for a hidden tab or session;
- `unavailable` (exit 1) while that tab already shows a Scrollback popup, or
  shows a transient popup such as Commands, which an edit never closes;
- `choose-directory-first` (exit 1) while that tab is still choosing its
  directory;
- `unknown-popup` (exit 3) when Scrollback is disabled.

Once the snapshot is written, it exits 1 with `popup-command` when the editor
is missing and with `popup-start` when the snapshot or the editor cannot
start. Either way no popup is shown and Eon removes the snapshot, except when
Eon cannot confirm that a `popup-start` stopped the editor: it may then still
run, and Eon keeps its snapshot until it ends. When the popup ends, Eon
removes the snapshot; see [Editor](CONFIGURATION.md#editor) for keeping your edits.

A read, or an edit, exits 1 with `terminal-access-off` when
[`[agents] terminal_access`](CONFIGURATION.md#configuration-behavior) is
`"none"`, `terminal-ended` once the program has exited, and
`terminal-unavailable` when the terminal's Orbit refuses or does not answer
within 2 s. An unknown target exits 3 (`unknown-target`). No failure carries
terminal content. Any process running as you can read terminals this way:
`terminal_access` is a preference, not a security boundary.

### Sending to a terminal

`eon pane send TARGET -- TEXT` pastes TEXT into a pane or popup, named as for
`eon pane read`, in any session of the window, as if you had pasted it there:
bracketed when the program enabled bracketed paste. `--stdin` reads TEXT from
standard input instead, once it has found the target window, so a failure to
find it is reported before standard input is read. `--enter` then presses
Enter, after
[`[agents] send_enter_delay_ms`](CONFIGURATION.md#configuration-behavior)
(300 ms by default) so the program takes the whole paste first; `--enter`
without TEXT, or with empty TEXT, presses Enter alone. A line break in TEXT
never submits it. Sending changes no selection, shown session or popup,
scroll position or focus; a hidden popup receives the text and stays hidden.

Keep each send short and point the program at a file for long material. Some
agents treat long pastes as outside material: Claude Code marks pastes over
800 characters or 3 lines that way.

With `--json` the reply names the target and `caller` as `eon pane read` does,
with `generation` and `window`, `bytes` (TEXT's length, 0 for Enter alone),
`bracketed` (null for Enter alone) and `enter`. Human output prints nothing on
success.

Orbit refuses each paste or Enter while a person's typing is queued or
within [`[agents] send_typing_quiet_ms`](CONFIGURATION.md#configuration-behavior)
(2000 ms by default). Wait for the quiet interval, or use `--force` to bypass
only the typing check; 0 disables it. CLI input never extends this interval.

A send refuses, with nothing sent:

- `invalid-text`, exit 2: TEXT is not UTF-8, or holds ESC, DEL or another
  control character other than tab, line feed and carriage return;
- `input-too-large`, exit 2: TEXT is over 64 KiB;
- `person-typing`, exit 5: a person is typing in the target;
- no TEXT and no `--enter`, exit 2;
- `unread-stdin`, exit 2: neither `--stdin` nor TEXT, and standard input is a
  pipe, socket or file that already holds text, as in `echo hi | eon pane send
  p1 --enter`. Add `--stdin` to send that text, or give the command empty
  input to press Enter alone (see the loops below). A terminal, `/dev/null`,
  an ended input, or a pipe or socket still empty after 100 ms is not read;
- `unbracketed-newline`, exit 1: TEXT has a line break and the program has not
  enabled bracketed paste;
- `terminal-access-off`, exit 1, when `[agents] terminal_access` is `"none"`.

It also exits 1 with `terminal-ended` once the program has exited, and with
`terminal-unavailable` when the terminal's Orbit refuses or does not answer
within 2 s. A send that timed out may still arrive. When the paste lands and
the Enter then fails, the command exits 1, the text stays in the program, and
the JSON reply carries `enter: false` and the Enter's `error`, including
`person-typing`. Do not retry the paste: it would duplicate the text. An unknown
target exits 3 (`unknown-target`).

A loop that presses Enter in several panes can hand `eon` its own input: a
Nushell closure passes its element to the first external command, and in a
bash `while read` loop the command inherits the rest of the list. Give the
command empty input so it presses Enter alone:

```nu
["p1" "p2"] | each {|pane| null | eon pane send $pane --enter }
```

```bash
printf '%s\n' p1 p2 | while read -r pane; do
  eon pane send "$pane" --enter </dev/null
done
```

**Sandboxed agents.** A sandboxed agent that may connect to Unix sockets can
drive Eon, and `eon pane create` and `eon pane send` then run commands outside
its sandbox. Codex with `sandbox_workspace_write.network_access = true` and
Claude Code with `sandbox.network.allowAllUnixSockets = true` allow this; both
tools' default sandbox modes block it. `[agents] terminal_access` is not a
security boundary.

**Codex's shared daemon.** By default the Codex TUI runs its commands through a
shared background server, which keeps the environment of the terminal that
started it. Its `eon` commands then carry that terminal's `EON_TERMINAL`, or
none when it started outside Eon, so `eon pane create` and the rest act for
that terminal, or for the shown session, not for the pane Codex runs in.
Once that terminal has ended they fail under the caller-discovery rules above.
An agent's `eon quit --json` through a daemon started in Eon quits the window
of the terminal that started the daemon, without asking. Start Codex in Eon
with `codex --no-daemon`; the Agent popup's `auto` command already does.

### Exit status

`eon` and `eonbase` commands exit with the same status with or without
`--json`:

| Status | Meaning |
|---:|---|
| 0 | Success, including a Stop or close confirmation declined at a terminal |
| 1 | The command was understood but refused or failed, including a confirmation that got no yes when stdin is not a terminal |
| 2 | Invalid command or arguments, such as a malformed tab or window ID |
| 3 | The named tab, pane, terminal, popup, generation or window was not found, or a window ID prefix matched several windows |
| 4 | The supervisor is not running or did not answer; outside Eon, run `eon` first |
| 5 | `pane send` refused because a person is typing; nothing was sent |
| 130 | Interrupted with Ctrl+C |

The `code` in an `error` result, such as `unknown-tab`, names the failure more
precisely. After editing configuration, run `eon config check`: it checks settings
Eon would reject at startup or during later use, using this build's defaults.
It needs no supervisor and creates no directories; a missing `config.toml` uses
the defaults. It exits 0 when valid or 1 on a read or validation failure;
invalid arguments exit 2. `--json` prints `{path, valid, error}` on stdout with
no stderr: `path` is the absolute file path (null if root discovery failed),
and `error` is null or `{code, detail}`. `invalid-configuration` means a parser
or settings error; `unreadable-configuration` means discovery or reading failed.
Consumers allow additional fields. Success does not mean every key was recognised:
unknown/ignored settings keep their usual behavior, and this command reports no
warnings. It does not check availability of external programs, files or services.

`eon update --install` exits 0 for installed, unchanged or a human
decline; 1 for failed checks/builds/selection, invalid configuration or absent
noninteractive confirmation; 2 for invalid arguments; 3 for a missing target;
4 for a typed refusal; and 130 for a handled interrupt. It reports the actual
selection after failures and cancellation. `eon update --check` exits 0 once
it has checked, even when it reports that Eon may not install the update;
1 when GitHub could not answer; 2 for invalid arguments; 3 when the named
profile or entry does not exist; and 130 for a handled interrupt. Both modes
stop their owned helper processes when interrupted.
`eon quit --all` and `eon window stop previous|all` exit with the
largest status among their attempts. `eon window attach|stop ID` exits with
the selected window's status, and `eon window attach ID` exits 4 once that
window has ended. An `eon window attach` that runs the `eon` of another build
exits with that `eon`'s status. Commands that run another program instead
exit with its status:
`eon anima` with Anima's, and `eon run -- COMMAND` or `eonbase -- COMMAND` with
the initial command's once it has ended.

### Agents in Eon

Eon packages an `eon` skill that teaches agents the CLI, how to find their own
pane or popup through `eon session --json`'s `caller`, and how to leave your
focus alone. An agent in an Eon terminal reads it with `eon --skill`, which
prints the skill of the running build and needs no setup; `eon --help` ends
by pointing agents there. To have an agent load the skill on its own, install
it for the agents you use. For a `nix profile` install in
`~/.nix-profile`, run the relevant commands below; substitute your profile path
if you installed with `--profile`. The links follow profile upgrades.

Claude Code:

```sh
mkdir -p ~/.claude/skills
ln -s ~/.nix-profile/share/eon/skills/eon ~/.claude/skills/eon
```

Codex:

```sh
mkdir -p ~/.agents/skills
ln -s ~/.nix-profile/share/eon/skills/eon ~/.agents/skills/eon
```

Remove a skill by deleting its link: `rm ~/.claude/skills/eon` or
`rm ~/.agents/skills/eon`.

An agent's sandbox does not contain what it does through `eon pane create`,
`eon pane read` and `eon pane send`, and Codex needs `--no-daemon` in Eon; see
[Sending to a terminal](#sending-to-a-terminal).

Optionally add a session-start hook. Merge the following `SessionStart` entry
into `~/.claude/settings.json`, preserving your other settings and hooks:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "sh -c '[ -n \"$EON_TERMINAL\" ] || exit 0; printf \"This agent runs in an Eon terminal; the eon skill explains the eon CLI; leave focus where the person put it\\n\"; eon session 2>/dev/null || true'",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

For Codex, merge the same entry into `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "sh -c '[ -n \"$EON_TERMINAL\" ] || exit 0; printf \"This agent runs in an Eon terminal; the eon skill explains the eon CLI; leave focus where the person put it\\n\"; eon session 2>/dev/null || true'",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

If you already keep hooks in the `[hooks]` table of `~/.codex/config.toml`, add
the entry there instead; Codex warns when both that table and
`~/.codex/hooks.json` contain hooks.

Codex runs a user hook only after you trust it in `/hooks`. Both examples run
the same command through `sh -c`, including when Codex invokes hooks through
`$SHELL -lc` with nushell as your login shell. The hook stays silent outside
Eon. Inside Eon it prints one sentence and the human `eon session` output,
including the caller line; a failed query does not fail the agent's start.
Remove the entry to remove the hook.

When an agent's shell starts Eon, such as `eon` run by Claude Code, Eon
removes a fixed list of agent session, nesting-marker and credential names,
such as `CLAUDE_CODE_CHILD_SESSION` and `CLAUDE_CODE_MESSAGING_TOKEN`, from its
terminals, popups, Venus and quota reader. A Claude Code started in an Eon
terminal then saves its transcripts, and the launching session's messaging
token stays with that session. EON-C9 in
[`CONTRACTS.md`](CONTRACTS.md#eon-c9--managed-shell-environment) lists exactly
which names. Eon also removes the settings Claude Code and Codex give the
commands they run, such as Claude Code's `GIT_EDITOR=true`, which leaves a
bare `git commit` with an empty message, and Codex's `NO_COLOR=1` and
`PAGER=cat`, but only with the agent's exact value while its marker
(`CLAUDECODE` or `CODEX_CI`) also arrived; any other value passes through.
Codex also forces the locale to `C.UTF-8`; Eon gives terminals your desktop
session's `LANG`, `LC_CTYPE` and `LC_ALL` back from the systemd user manager,
and where there is none (such as on macOS) it keeps `LANG=C.UTF-8` so
terminals stay UTF-8. Eon cannot tell your own identical value from the
agent's: if you set `PAGER=cat` or `NO_COLOR=1` yourself and start Eon from
such an agent shell, command panes, popups and the Editor lose it. Set it in
your shell configuration, or in the command itself (`env PAGER=cat COMMAND`),
to keep it. Other variables the agent's shell set, such as
`CLAUDE_CONFIG_DIR` and API keys, still reach Eon terminals. Only a
supervisor started after you update Eon removes these names; detaching and
reattaching a window, or recovering it, keeps the old environment.

### Accelerando glyphs

This animation is available in builds that include Accelerando. Without a
recorded redistribution grant, the Linux release archive omits the animation
and its fonts; Game of Life stays available. See [release routes](INSTALLER.md#release-assets).

In Eon's native terminal, `eon anima accelerando` shows the 8,192-glyph alien
alphabet in long green trails; add `--symbols` to mix in letters, digits and
punctuation. Add `--big` for large, solid block glyphs; combine `--big --symbols`
for matching large letters, digits and punctuation. Add `--bold` for heavier
native glyphs; it combines with either option. Large solid blocks already fill
their cells. Random playback includes Accelerando. Eon bundles Regular and Bold
Accelerando Terminal faces; other terminals need these fonts for native-size
playback. There is no substitute alphabet.

## eonbase

Install eonbase when one exact command needs a native terminal surface without
Eon's sessions or managed environment:

```sh
nix profile add .#eonbase
eonbase -- COMMAND...
eonbase --no-decorations -- COMMAND...
```

Its lifecycle commands mirror Eon's:

```sh
eonbase generations
eonbase attach [GENERATION]
eonbase stop GENERATION
```

Eon and eonbase use separate runtime namespaces. eonbase keeps native window
decorations by default and accepts `--application-id ID` for an approved
composition.

## Troubleshooting and reporting

- **Nix cannot fetch an input:** check access to GitHub and retry. Eon
  fetches only external inputs; Orbit, Venus and Eon Runtime build
  from this repository.
- **A session command reports a missing supervisor:** run `eon`, then retry.
- **The window closed but commands remain:** this is detach behavior
  (Alt+Shift+D, an eonbase window's close, or a lost window). Use
  `eon window attach` to return, or `eon quit` or the notification's Quit to
  end them.
- **A window closed on its own:** each Eon or eonbase supervisor keeps a log
  in `$XDG_STATE_HOME/eon/logs` (`~/.local/state/eon/logs` by default), named
  `GENERATION-SECONDS-PID.log`. It records when the window started and how it
  exited, with its error output and any crash backtrace, when terminals exit on
  their own, and how long each tab close took. Eon keeps the newest 20 logs
  plus those of running supervisors, and stops a log at 1 MiB. Orbit's own
  diagnostics go to `$XDG_STATE_HOME/eon/logs/orbit`
  (`~/.local/state/eon/logs/orbit` by default), named
  `GENERATION-TERMINAL-RUN.log`; Eon keeps the newest 100 plus those of running
  Orbit hosts.
- **A command works in Yazelix Nova but not in Eon:** Eon's managed Nushell
  and Bash load your own shell configuration (`~/.config/nushell` and
  `~/.bashrc`), while Nova's Nushell reads `~/.config/yazelix/nu` in place of
  `~/.config/nushell/config.nu` and `env.nu`. Environment variables, secrets
  and hooks set in your own configuration apply in Eon terminals, so look
  there first.
- **A replacement window rejects configuration:** fix `config.toml` and run
  `eon window attach`; the existing terminals remain alive.
- **Graphics fail outside native Wayland or on unproved hardware:** that
  environment is outside the supported boundary.

Report reproducible failures in [GitHub Issues](https://github.com/Yazelix/eon/issues/new?template=bug_report.yml).
Run `eon report` from the window that failed, read it, then paste it. It prints
the bug form's fields as Markdown: `eon versions`, the window from
`eon window ls --json` without the window's name or busy terminals' names
(outside Eon, the `default` window), the install route, distribution,
compositor, configuration warnings and the last 40 lines of the newest
supervisor log, which it names. A part it cannot determine reads `unknown`. It
shows paths under your home as `~` and removes long environment values (8 bytes
or more), except `XDG_CURRENT_DESKTOP`, `XDG_SESSION_TYPE`, the XDG base
directories under your home (shown as `~` paths) and Eon's `EON_*` identifiers,
and writes your user name as `[user]`. It reads no terminal content, but the log
tail may still hold other private text, so read it before you share. Fill in the
GPU, the exact steps, and the expected and actual results yourself, and attach
the whole log when asked. Commands offers it as **Show bug report**.

### Naming a pane

`eon pane rename p1 'Review pane labels'` pins a name to that pane. Use
`eon pane rename p1 --clear` or an empty name to restore automatic naming.
Names are trimmed, limited to 80 Unicode characters, and reject control and
invisible format characters. `eon pane rename p1 -- '--clear'` pins an
option-looking name; put `--json` before `--` to request JSON with a literal name.
Rename targets the explicit pane in the addressed session, including background
tabs, and preserves selections and focus. Commands offers **Rename pane** and
**Clear pane name** for the popup tab's selected pane; Enter on an empty name
clears it, and Escape cancels. Pins follow pane moves and survive presentation
detach/reattach, but end with the pane or supervisor and are not recovered.
`eon session --json` and action replies expose each pane's full `label` and
nullable `pinned_name`, including panes without agent-attention records.

Add `--tag TAG` to a rename to set a separate plain-text work identifier:
`eon pane rename p1 review --tag eon-xvz5c`. Tags are trimmed and use the same
80-character / 320-byte plain-text bounds as names. `--tag ''` clears the tag;
ordinary rename and `--clear` preserve it. Supply NAME when using `--tag`, even
an empty name to restore automatic naming while setting the tag. `--clear`
cannot be combined with `--tag`. A tag value may start with `-`; place `--tag TAG`
before the literal `--` when NAME looks like an option. Invalid names or tags
leave both fields unchanged.

Pane JSON exposes nullable `tag` beside `pinned_name`, including in subscription
snapshots, gaps and pane events. A tag change emits `pane-renamed`. Tags follow
the same lifetime as pins and are not interpreted as Beads IDs. Setting a tag
requires a fresh window using the matching CLI, Runtime and Venus build;
existing windows keep their original build and terminals.

## Subscribe to events

`eon events --json` prints one NDJSON snapshot, then events, gaps and an end.
It observes the whole window without moving focus. Use `--window ID` for an
explicit window; `EON_TERMINAL` is validated as for other session commands.

```sh
eon events --json --tab t2 --kind attention --state needs-input
eon events --json --session s1 --target p2
eon events --json --after STREAM:SEQ
```

Repeat `--session`, `--tab`, `--target`, `--kind` or `--state`: values within
one flag combine with OR, different flags with AND. Kinds are `session`, `tab`,
`pane`, `popup`, `attention`, `queue` and `busy`. States use `needs-input`,
`error`, `finished`, `working`, `background`, `idle` and `unavailable`.
Attention matches its previous or current state. A target follows its terminal
when a popup becomes a pane. Session selectors resolve once by ID or exact name.

The first line gives `stream`, `seq`, `resumed`, `generation`, `window`, `caller`
and state (`sessions`, `tabs`, `attention_queue`, `busy`). Save `STREAM:SEQ` to
resume retained events; a successful resume has null state, otherwise a fresh
snapshot. Sequence numbers advance before filtering, so skips are normal.
A `gap` with reason `overflow` replaces state with its `snapshot`. Handle it
before subsequent events. The retained log holds at most 1024 events / 8 MiB;
pending events are bounded by 1 MiB. A supervisor serves at most 32
subscribers. A subscriber stalled for 30 seconds closes; progress is measured
in write units of up to 4 KiB.

Human refusals go to stderr; JSON refusals go to stdout.
Without `--json`, output is one human line per message; attention shows only
state, app and name. JSON includes the full attention record, including untrusted
title and message text. End reasons `ended` and `targets-gone` exit 0;
`too-slow` exits 1 only when its complete end arrives, otherwise connection loss
exits 4. Refusals exit 1 for `subscriber-limit` or `ambiguous-session`, 3 for
missing/stale targets or callers; invalid arguments exit 2, unreachable or
incompatible supervisors or other EONW failures exit 4; interrupt exits 130.
Closed stdout exits 0.
`eon pane wait` is planned under `eon-ciwn.9`.

Use the stream as a change feed when filtering by session, target or state;
read `eon session --json` for complete current state. A tab-filtered stream
keeps its named tabs current but does not rebuild session-wide selection or
window order. Add `--kind queue` to receive the whole queue on each change.
See [EON-C39](CONTRACTS.md#eon-c39--session-events-from-the-cli) for the
snapshot and event field sets and the exact reconstruction guarantees.
