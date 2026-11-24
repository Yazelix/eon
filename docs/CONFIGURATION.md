# Configuration

Settings live in `config.toml`; `eon config path` prints its full path, normally
`~/.config/eon/config.toml`. `eon config dir` prints the configuration root
that holds it, which also holds the managed tools' configuration directories.
After editing, run `eon config check` to check for settings Eon would reject at
startup or during later use. It needs no running Eon and changes no state; a
missing file uses this build's defaults. It exits 0 when valid, 1 on read or
validation failure, and 2 for invalid arguments. Errors name the file and the
key or source location when available. `eon config check --json` returns
`{path, valid, error}`, with `error` null or `{code, detail}`; see
[Exit status](USAGE.md#exit-status) for the error codes. Success does not mean
every key was recognised: unknown/ignored settings retain their usual behavior,
and this command reports no warnings. It does not check availability of external
programs, files or services.

This example chooses a font, cursor preset and 100x30 grid; it is not the
empty-configuration default:

```toml
[bar]
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "claude_quota_scoped", "cpu", "ram", "disk", "clock", "version"]

[terminal]
background_opacity = 0.80
background_blur = true
padding_color = "extend"
pane_frames = true
focus_color = "custom:#b207fb"
border_width = 1
focus_border_width = 1
cursor_trail_color = "preset:ice"
cursor_trail_duration = 1.5
font_family = "Iosevka"
font_size = 16
line_height = 1.125
columns = 100
rows = 30

[shell]
command = ["eon-nu"]
starship = true
zoxide = true
atuin = true
carapace = true
mise = true

[anima]
enabled = true
style = "random"
duration_seconds = 3

[session]
warning_close_last_tab = true
animate_working_badge = true
number_keys = true

[sidebar]
enabled = true

[attention]
notify = true
notify_names = true

[agents]
terminal_access = "window"
send_enter_delay_ms = 300
send_typing_quiet_ms = 2000
```

## Agent commands

`eon agent` reads `agent.toml` in the directory printed by `eon config dir`:
`$EON_CONFIG_HOME/agent.toml`, otherwise `$XDG_CONFIG_HOME/eon/agent.toml`
(XDG_CONFIG_HOME must be absolute), otherwise `~/.config/eon/agent.toml`.
This is a separate file with rota's existing TOML schema; it is outside
`eon config check`, `defaults` and `schema` until the clean port.

Only while `agent.toml` is absent, configuration falls back to `$ROTA_CONFIG`,
then `$XDG_CONFIG_HOME/rota/config.toml`, or `~/.config/rota/config.toml` when
XDG_CONFIG_HOME is unset. A present Eon file wins even when ROTA_CONFIG is set;
an unreadable or invalid Eon file exits 2 instead of falling back. The fallback
is transitional and is removed with rota retirement (`eon-eon-agent-lr8sx.20`).
No command migrates or rewrites these configuration files.
The adapter still needs a resolvable Eon configuration home: if HOME,
XDG_CONFIG_HOME and EON_CONFIG_HOME are all unset, set HOME or EON_CONFIG_HOME
even when ROTA_CONFIG points to the fallback file.

Start from the [example](../eon/crates/eon-agent/config.example.toml), replacing
its placeholder paths and commands. Paths are literal; `~` and environment
variables inside TOML are not expanded. `state_file` and `log_file` are required
distinct paths; keep them outside repositories. Backend array order is
admission order: each launch takes the first available backend. Each backend
specifies roles (`implement`, `review`, and `lead` to accept lead launches), a
guard and argv commands; quota, account
checks, caps, preparation, trust settings and headless run commands retain
rota's schema. No backend or provider is configured implicitly.

The example requests agy's sandbox and Codex's workspace-write sandbox with
on-request approvals. Codex TUI commands also use `--no-daemon`; headless
implementation/review commands keep workspace-write/read-only respectively.
Other backends retain their own permission settings. No example command opts
into permission bypass. Sandboxing can deny host-socket access and writes to
external state directories. See the
[backend permission guidance](../eon/crates/eon-agent/README.md#configuration)
for state-directory access and deliberate custom bypass choices. A custom
bypass backend's configured name appears in status; that name is a label,
not an Eon security check.

The [moved crate's configuration reference](../eon/crates/eon-agent/README.md#configuration)
documents every field. Its unchanged defaults include implement/high for
launch, implement/current directory for run, reader timeout 30 seconds, refresh
timeout 90 seconds, two slots, child nice increment 19, empty slot environment
and classes, cleanup min_idle `2h` and no store prefixes. Guard/exec behavior,
run timeout/cleanup bounds, diagnostics and ROTA_* controls remain as in rota;
this move adds no new preference or default backend.

Native quota readers use `parser = "eon_codex"` or `"eon_claude"` and read
Runtime's session facts in process. Native readers ignore optional `command`;
rota still reads that field when both tools share a configuration. Custom
parsers require `command`. All readers use `timeout_seconds` (default 30): a
native read shares one deadline across discovery, connection and inspection,
and the caller's window gets at most half of it. A provider that window does
not supply (it runs another EONW, cannot be reached or answer in time, or has
no facts for that provider) is read from the quota store in
`$XDG_STATE_HOME/eon/quota` without calling the provider: a recorded Claude
failure still holds, an unreadable failure record refuses the reading, and a
stored Codex reading counts only while at most 120 s old. The store assumes one
account per provider for each Eon state directory, as the quota chips do. The
native reader never invokes an incompatible generation's recorded launcher.
Each native reading names its source (`reading.source.via` in
`eon agent status --json`: `window ID` or `store`), and a provider no source
has names both reasons. Native readers
keep the same fresh-only admission, role thresholds and optional refresh retry;
collection and `eon session --json` quota output are unchanged. Backend provider
credentials do not change the supervisor's selected quota account.

Set `parser = "codexbar"` in `[backends.quota]` to use one CodexBar CLI
account for a backend. With standalone `rota`, put the CLI on `PATH` or
give its absolute path as the first command argument.

```toml
[backends.quota]
parser = "codexbar"
command = ["codexbar", "usage", "--provider", "codex", "--source", "cli", "--format", "json", "--json-only"]
```

The installed `eon` wrapper sets `EON_CODEXBAR` to its bundled CLI.
Use this command to resolve it when the check runs.

```toml
[backends.quota]
parser = "codexbar"
command = ["sh", "-c", 'exec "${EON_CODEXBAR:?EON_CODEXBAR is unset}" "$@"', "codexbar", "usage", "--provider", "codex", "--source", "cli", "--format", "json", "--json-only"]
```

For Claude, use `--provider claude --source oauth`. Use `eon_claude` for
the Claude account Eon already polls, so agent commands share Eon's polling
and back-off. Direct reads also run during status and dry runs. A rate-limited
read, provider error, nonzero exit or stale result holds. CodexBar owns
credential access for these commands.

Use `five_hour` and `seven_day` in limits and `absent_windows`. Claude's
named `claude-weekly-scoped-<id>` extras use `seven_day_<id>`. Quote keys
with non-ASCII characters in TOML, for example `"seven_day_é" = 50`.
Configure both the general and scoped weekly caps when they apply. A cap
that CodexBar reports only in Claude's unnamed `tertiary` window has no
model key. A limit on a missing named window holds unless declared absent.
With `"*"`, CodexBar requires both base windows unless declared absent and
checks every usable reported model window. An unusable or missing model
extra requires an explicit `seven_day_<id>` limit to hold for that cap.

Upgrade every `rota` or `eon agent` binary that reads a shared config before
enabling this parser. Older binaries reject the file. When switching from
a native reader, replace duration keys such as `300m`, `10080m` and their
indexed variants in limits and `absent_windows` with the corresponding
named keys. A native duration key can cover several windows, so list each
applicable key. Restore the previous config before rolling back a reader.

`status --json` keeps the configured account in `reading.source.account`.
When CodexBar identifies the account, `reading.source.identity` includes
the available `account_id`, `account_email` and `account_label` fields.
The ledger and decision log use the configured account.

`eon agent status` prints `config: PATH` before backend rows.
`eon agent status --json` keeps rota's backend array and adds `config_file`
to every backend object (a string, or a byte array for a non-UTF-8 path).
Other output and record formats are unchanged.

`eon agent launch --popup [--only BACKEND] [--dry-run]` uses implement/high
admission and popup resume commands, with no prompt-file operand. Ordinary
launch still requires its prompt file. Each backend may set an optional
`popup_command = ["my-agent", "--resume"]` alongside `name`, `roles` and
`guard`. Its argv must have a nonempty executable, contain no NUL, `{prompt}`,
`{name}`, `{add_dirs}` or `{overlay}`, and may use `{cwd}` inside any argument
but the program. It runs under that backend's
existing environment, checks, limits and preparation.

Optional top-level `popup_backend = "main"` pins popup starts to a configured
implement backend or the configured fallback. Omit it to keep dynamic
implement/high selection. `--only BACKEND` overrides the pin with its existing
role restrictions. A refused pin never selects another backend. Ordinary
launch and run selection are unchanged.

Optional top-level `wait_retry_seconds` (an integer from 1 to 3600, default
60) is the longest sleep between the attempts of `launch --wait` and
`run --wait`; a waiter re-reads it on every attempt. Builds before it was added
reject the whole file when the key is present. Upgrade every `rota` or
`eon agent` binary that reads a shared config, including a running window
supervisor's Agent popup launcher, before setting it, and remove it before
rolling back.

Each backend may set `account = "NAME"`, the subscription account it draws
on; it defaults to the backend's `name`. Backends that use one account under
different models or wrappers declare the same `account`. A backend that sets
`limits` must declare `account`, may not set `allow_unknown_quota`, and admits
one launch or run per quota observation of that account: its reader must
report `observed_at` (`claude_text`'s `observed_at=MS` token, a top-level
`"observed_at"` in `claude_json` or `remaining_json`, or the native snapshot's
time). Rota converts CodexBar's RFC 3339 `usage.updatedAt` to Unix
milliseconds. Spent observations are kept in `admissions.json` beside
`state_file`.
`[backends.quota] max_age_seconds` (a positive integer, default 600 on a
backend that sets limits or uses CodexBar) holds a reading older than that
many seconds, even an unspent one. Each quota hold names the account and,
for a limit, the window, its value, the rule and its threshold.
For a backend with `parser = "eon_claude"`, when Eon's quota facts report
that a Claude read failed after the last good reading, that reading is
stale. Status and launch lines show it as
`stale 18m (last read failed: provider_error)` after the window values, and
JSON readings carry `source.stale` as `{"age_ms", "failure"}`. On a backend
that sets limits, a stale reading still admits while it is under 30 minutes
old when admission reads it, whatever `max_age_seconds` says. Limits and
the one-launch-per-observation rule apply to it as to any reading. At 30
minutes or older it holds after one more read of Eon's facts, without
running `refresh`. A Claude failure older than the last good Claude reading
does not hold; any other failed reading holds as before, including any
failure beside Codex facts. The 30-minute bound is fixed: Lucca's
approved rule (2026-11-19) names it.
`[backends.quota] absent_windows` (default empty) lists windows the account
does not have: a limit naming a missing window holds unless it is listed. A
key `DURATIONm` without an index, in limits or `absent_windows`, covers every
native window of that duration. `limits.lead` and `run.commands.lead` are
errors. `limits.ROLE.allow_missing` is an unknown key: a file that sets it
fails to load, and the error points to `absent_windows`.

Resume reads the admitted backend's native store. Claude uses
`CLAUDE_CONFIG_DIR`, or `~/.claude` under its effective home when that variable
is absent. To keep the popup on sessions in `~/.claude`, pin the configured
backend owning that store, for example the `main` fallback in the
[example configuration](../eon/crates/eon-agent/config.example.toml), whose
`unset_env = ["CLAUDE_CONFIG_DIR"]` clears an inherited alternate store.
Its guards, account checks, quota and caps still apply. Eon does not search
or merge histories across stores or change accounts after admission.

When `popup_command` is absent, the configured high command's executable
basename selects its popup argv: `codex` adds `resume --no-daemon`, `claude`
adds `--resume`, and `grok`, `opencode` and `pi` add nothing. The executable
path itself is retained. Other executables are unavailable in popup mode
until given `popup_command`; normal launch remains available. Popup mode
does not copy high-command model or permission flags into these defaults.
Explicit popup argv remains your command choice. A busy admission lock
holds immediately (exit 75); popup mode takes no `--wait`.
On a real popup-mode failure with terminal input and stderr, the diagnostic
stays in that terminal until Enter or EOF, then returns its original status.
It uses the existing fixed `Press Enter to close` dismissal wording. Dry-run,
nonterminal invocations and ordinary launch still return immediately.

### Agent projects

`[projects.NAME]` holds `retire`'s directories (`repo`, `agents_root`, `done`)
and, optionally, a profile that `launch` and `run` follow (EON-C40 clause 6):

```toml
[projects.tide]
repo = "/home/me/src/tide"
agents_root = "/home/me/.cache/tide-agents"
max = 2                          # live implement and review agents in total

[projects.tide.backends.codex]   # a configured backend's name
max = 1

[projects.tide.roles.implement]  # implement, review or lead
max = 2                          # not allowed on lead
guard = ["/usr/local/bin/machine-free"]

[projects.tide.roles.implement.overlay]  # by command executable name
codex = ["-s", "danger-full-access", "-a", "never"]
claude = []
```

Every profile key is optional, and an absent one changes nothing:

| Key | Default when absent |
| --- | --- |
| `projects.NAME.max` (at least 1) | no total cap |
| `projects.NAME.backends.BACKEND.max` (at least 1) | no cap on that backend |
| `projects.NAME.roles.ROLE.max` (at least 1; not `lead`) | no cap on that role |
| `projects.NAME.roles.ROLE.guard` (argv) | no guard |
| `projects.NAME.roles.ROLE.overlay` (executable = argv) | no overlay; the role picks any backend, unchanged |
| `[[backends]] add_dir` (argv with one `{dir}`) | none; that backend's launch commands take no `{add_dirs}` and no `--add-dir` |

- **Selection.** `--project NAME` selects a project from anywhere, and an
  unknown name is refused (exit 69). Without it, the project is the one whose
  `repo` holds the working directory (for `run`, `--cwd`): any folder of the
  checkout, or of a linked worktree, a submodule or a worktree of a bare
  repository, found from `.git` files without running git. `repo` is compared
  after resolving symlinks. Two projects on one repository are allowed for
  `retire`, but selecting by folder then refuses and asks for `--project`. A
  folder in no project selects none and behaves as before. The person's popup
  (`--popup`, which refuses `--project`, `--name` and `--add-dir`) never
  selects a project. The selected project is named on stderr and in the
  decision record.
- **Live agents.** An agent counts as live while the process its admission
  started still runs as the same user: the agent itself for `launch`, the
  supervising `run` or its run's leader for `run`, each recorded by PID and
  start time in `agents/` beside `state_file` before it starts. Processes an
  agent leaves behind do not count. Records of finished agents are removed by
  later admissions. Leads write no record and are never capped.
- **Holds.** The total `max` and the role's `max` hold before any backend is
  read, naming the live PIDs. Then the role's `guard` runs in the working
  directory with a fixed 15 second limit, like backend guards. A failing guard
  holds with the first non-blank line of its stderr, cut to 200 characters with
  control characters replaced. This is the one probe output copied to
  diagnostics. Finally, a backend at its project `backends.NAME.max` is skipped
  with that reason. A hold exits 75 and is written to the decision log with
  `project` and `hold`.
- **Overlays.** A role's overlay adds arguments at the `{overlay}` argument of
  every command that role reaches whose executable it names: level commands of
  backends serving the role, the implement fallback, and `run.commands.ROLE`.
  Every account running that executable gets the same overlay, so none is
  pinned. A role with overlays picks only executables it names; `claude = []`
  allows claude unchanged. Configuration refuses an overlay whose commands lack
  `{overlay}`, and overlay arguments may use `{cwd}` and `{name}` (and run's
  `{brief}`, `{report}` and `{effort}`).

Launch level commands accept these placeholders:
- `{prompt}`: exactly one whole argument.
- `{cwd}`: the working directory, filled inside any argument.
- `{name}`: the `--name NAME` value (1-64 letters, digits, `.`, `_` or `-`),
  filled inside any argument.
- `{add_dirs}`: one whole argument. It repeats the backend's `add_dir` template
  once per `--add-dir DIR`, for example:

  ```toml
  [[backends]]
  name = "main"
  add_dir = ["--add-dir", "{dir}"]   # or ["--add-dir={dir}"]; exactly one {dir}
  [backends.commands]
  high = ["claude", "-n", "{name}", "{overlay}", "{add_dirs}", "--", "{prompt}"]
  ```
- `{overlay}`: one whole argument.

`run` commands take the same `{name}`, `{add_dirs}` (through
`[backends.run] add_dir`) and `{overlay}`. No placeholder may be the program.
A backend whose command needs `--name`, takes no `--add-dir`, or has no overlay
for the project role is skipped with that reason; when none remains the command
is refused (exit 69), since waiting cannot change configuration.

### Remote heavy builds

`eon agent slot run` can send an explicitly permitted build to Boat. Local
execution is the default; the Boat CLI must already be installed and signed in
on the calling machine. Its account credentials stay there.

`eon agent slot --config FILE run -- COMMAND...` and `slot --config FILE ls`
use a slot-only configuration override. Other agent commands keep their normal
lookup; the override is not exported into the command's environment.

`slots.local_runner` and `[slots.boat]` act only under `eon agent`: standalone
`rota` warns that they need `eon agent` and runs every slot command on a local
slot. `slots.local_runner = []` keeps native local admission. A trusted nonempty argv
prefix delegates local admission to that host-only command, without holding a
native slot or remote lease. It receives original queue age/class through
`ROTA_SLOT_QUEUED_NS`/`ROTA_SLOT_CLASS`, with `HEAVY_DIR` from `legacy_dir` and
front/quiet flags. The delegate must preserve that age and acquire local
capacity before executing its supplied payload. The payload verifies a captured
remote source again after admission, then executes the original argv/cwd;
source mismatch/untracked additions exit125. Internal handoff variables are
removed before execution. A missing delegate fails rather than running unqueued.
Use private0700 state/queue/holders/boat/runs directories and0600 records;
unsafe existing state is refused, never repaired by changing shared Rota modes.
Delegated commands retain token-checked, locked ownership evidence so a nested
slot call reuses admission rather than waiting on its own target. Cancellation
is forwarded to the delegate's own process group. Terminal calls give that
group foreground access before exec and restore the caller's foreground group
on completion or cancellation, so terminal input remains usable.
With `legacy_dir`, remote admission requires its `boat-on` marker (absent by
default). Removing it moves waiting remote requests to local admission before
their next lease claim; it does not interrupt an already-started remote command.
The private fleet front reads the same opt-in marker before native dispatch.
Without `legacy_dir`, the marker is `slots/boat-on` beside `state_file`;
absence still selects local admission. No configuration implicitly enables it.

```toml
[slots.boat]
allow_shapes = []
environment = "eon-builds"
reserve_hours = 2
timeout_seconds = 3600
jobs = 4
max_parallel_leases = 1
```

`allow_shapes = ["nix-gate"]` permits Eon's repository-local `nix build`
gate targets `.#checks.x86_64-linux.eon-tests`, `.#default`, `.#no-helix` and
`.#bench`. The last target builds benchmark tools; running them stays local.
Accepted flags are `-L`, `--print-build-logs`, `--keep-going`, `--no-link`,
`--max-jobs 1|2` and `--cores 1..6`. Unknown arguments, shell commands, profile
operations, Update and desktop/GPU/benchmark commands run through the local
queue instead. A configured class does not make an unsafe command remote.
`--out-link` stays local because Boat outputs never become host paths; keep host
GC roots in a separate local step.

The other supported `allow_shapes` values are `cargo-build`, `cargo-clippy` and
`eon-test`. They permit `nix develop -c cargo build` (workspace/locked/offline/
release/all-targets and `-p NAME`), `nix develop -c cargo clippy` with the same
flags and an optional trailing `-- -D warnings`, and the pinned
`nix develop -c eon-test` harness (ordinary subsets, `-p NAME`, doctests and 1–4
test threads). Ignored, live, visual and benchmark selectors stay local; direct
Cargo test/nextest commands do not bypass Eon's test sandbox.

A permitted command may be wrapped in one `timeout DURATION`, where DURATION is
a positive integer with an `s`, `m` or `h` suffix and no leading zero. The
original argv, `timeout` included, runs unchanged. Boat's own deadline of
`timeout_seconds` minus one minute starts before recovery, upload, Nix
bootstrap and warmup, so the envelope may be at most `timeout_seconds` minus
31 minutes: 89 minutes at 7200 and 29 minutes at 3600. A longer envelope,
`timeout` options, `infinity`, fractions, other units and nested envelopes run
locally. If preparation still overruns, Boat's deadline ends the run as an
uncertain 125 with no local replay. A run stopped on the VM is cleaned up by
deleting the VM, because `timeout` keeps its command in a separate process
group.

`environment` must be a named builds-only environment other than `base`.
Create it with `boat env new eon-builds`, then set all passing controls off
before starting a VM:

```sh
boat env set eon-builds --safe-for-third-parties true --github false \
  --secrets false --boat-credentials false --agents-credentials false \
  --proxy-ai-credentials false
```

The environment must contain no repositories, variables or secret files.
Each VM explicitly selects it with credential passing disabled, and an actual
VM probe checks for credential variables, login files and Git helpers before
uploading source. No agent sessions run in these VMs.
No caller credentials reach the VM. Boat's own sandbox-scoped identity
(`ASCII_TOKEN`, `BOAT_ID`) is present on the raw VM and acts only on that
sandbox; only those exact names are allowed by the raw probe, without reading
or logging their values. Both names must be absent from the actual build,
which uses `env -i` and `bash --noprofile --norc`. A private result records
`build_identity_absent`; missing or false proof refuses success. See
[Boat access rules](https://docs.boat.dev/environments/access) and
[reserved names](https://docs.boat.dev/environments/secrets).
The raw VM may contain `/home/user/.config/ascii/boat/config.json` only as a
regular JSON object with exactly the string fields `api_url` and `channel`.
The probe checks this fixed security schema before every upload, without
returning or logging file values. Missing Python, malformed JSON, duplicate
or unknown fields, wrong types and nonregular files refuse the remote route;
all other caller credential files remain forbidden.
Requested commands clear inherited/ambient process capabilities and still use
the ordinary `eon-test` isolation harness.

`reserve_hours` is a finite nonnegative amount of included compute kept in
reserve. Admission uses the smaller of included plan time and any trial cap;
wallet credit never authorizes a paid start. Keep-going must be off. The
whole run's lifetime must fit above the reserve and before the trial/billing
boundary. `timeout_seconds` accepts 120–7200 and is also the VM's lifetime,
after which Boat stops it, so it bounds billed time. The run itself, setup
included, ends 60 seconds before it. Teardown after that (kill, log and result
copies, stop, final usage, delete) can take up to 240 seconds more, each step
on its own bounded budget. The credit guards reserve those 240 seconds.
Use 7200 for a first cold gate. `jobs` accepts 1–4 for either supported machine;
explicit permitted command flags retain their own values.

Boat uses the same heavy memory classes as local admission. Defaults are 6G
for ordinary jobs, 8G for Nix and Rune, and 4G for quiet runs. The shared
`mem-caps` file can change the defaults, and `HEAVY_MEM` overrides the limit
with a whole number from 1G to 12G. Quiet timing jobs stay local. A remote run
uses the smallest machine type that has a qualified image and whose ceiling
covers its cap: the default 4-vCPU/8-GB VM up to 6G, or the large
8-vCPU/16-GB VM up to the measured `slots.boat.large_max_memory_gib` ceiling.
Only large has a qualified image, so every remote run uses large. The ceiling
defaults to 0, meaning the large VM is unqualified, and accepts 0 or 8–12; set
it to at least 8 to route remote runs. A cap that only the default type covers
enters local admission with reason `image-unpinned`, and a cap above every
ceiling with `unqualified-ceiling`. Both sizes draw from the same lease pool. A large job
requires twice the default rate of included credit.

The guest checks the actual per-job limit, no swap, ancestor limits, a local
Nix store without an external builder, and spare memory before preparation.
Default requires 1 GiB spare and large requires 2 GiB. A failed headroom
check records a qualification hold for that type until the operator resolves
it. A verified job-limit OOM exits 137 with the class and cap. VM-wide OOM
and unknown OOM cause are reported separately with exit 125.

Only tracked source, including tracked uncommitted edits and staged new
files, is exported. Non-ignored untracked files require staging before remote
proof. The verified Git tree includes tracked issue data; Git history, local
configuration, credentials and absolute host cwd are not uploaded. Remote cwd
is the same repository-relative directory.
Export writes raw Git blobs and verifies their bytes and modes locally before
upload; local checkout/smudge filters cannot inject data into the upload.

Each remote run holds a separate Boat lease, leaving local heavy capacity free.
`max_parallel_leases` accepts integers from 1 through 3 and defaults to 1 when
omitted. Cooperating callers share a state directory and a slot configuration.
At capacity, eligible requests stay in the existing queue order. Queued
requests reread the limit at grant. If the file becomes unreadable or invalid,
they stay queued until it can be read and validated again. Reducing the limit
lets holders finish and makes
requests wait until occupancy falls below the configured limit. Quiet policy
still applies. Admission checks
included compute for the new run and live siblings. Upgrade every `eon agent`
reader of the shared file before setting this key. Remove the key before
rolling back to an older reader. Before remote execution, policy/connectivity or
budget refusal falls back through the local slot queue and reports a reason.
A dropped output stream does not end a remote run. When `boat exec --attach`
fails, Eon reads the command's status with `boat exec --status`, and
re-attaches while it still runs. Waits between attempts grow from 1 to 30
seconds, and an attempt held for a minute resets them. If the command has
ended, Eon reads its result as usual. After a loss, only that status, the run
deadline, the credit guard or a signal can end the run: output silence no
longer can. The run is killed as `remote-attach-lost exit=N status=unreadable`
only when every status read fails for 10 minutes. While a read is failing, the
status is read again every 30 seconds, also during a later attach. `N` is the
last attach's exit code, or `none` when that attach could not run or was
killed. Before any loss, 10
minutes without output still ends the run as `remote-no-progress`. A
re-attach replays the output from its start. Eon skips the bytes it already
showed, as a best effort, and marks each re-attach with one
`[eon agent: Boat stream re-attached at stdout=N stderr=M; ...]` line in the
live log and on rota's stderr; stdout carries only the command's own bytes.
The complete log is copied from the VM afterwards.
After execution might have started, lost contact reports uncertainty and
cleans up the owned VM without automatically running a local duplicate.
Unresolved prior work blocks new remote starts while a new invocation can use
the local queue. An unconfirmed create expires only after its recorded bounded
lifetime and a provider inventory proves no VM with that name exists.
Policy-eligible noninteractive local fallbacks retain private `runs/<id>/`
output and a local receipt with the reason, status and `output_complete` flag.
Terminal callers keep their inherited streams. Default-local and
policy-negative commands keep direct streams without this recording.

Every remote run starts one fresh VM from the qualified image pinned in Eon:
Boat named snapshot `eon-builds-nix-2-34-7-large-klt8h-v1` (environment
`eon-builds` version 2, large). It is not configurable; a new image needs its
own qualification and an Eon release. Before the start, the configured
environment must be that environment and version, and `boat snapshots` must
show exactly one row of that name with the pinned snapshot ID and source VM,
ready. The start uses `--from` that name with captures off. Before anything
else runs on the VM, it must be active, hold no creator logins, report the
pinned type, environment and version, still match that row, and hold the
recipe digest in the root-owned, mode-0444 file `/opt/eon-boat/recipe.sha256`.
A failed check runs locally with `boat-image-unverified check=NAME` (the
provider or guest cannot show the image) or `boat-image-mismatch check=NAME`
(it shows another one). After the start, the VM is deleted first and nothing
is copied from it; if deletion is unconfirmed, the run stays unknown cleanup.
Stopped VMs and snapshots are never reused: each VM is stopped for its final
usage and deleted after its run. `slots.boat.snapshot` is removed; a leftover
setting loads with a warning and has no effect.

`eon agent slot ls --json` identifies remote holders with `where: "boat"` and
shows the latest Boat receipt in `boat`. `boat_lease_held` reports whether any
lease is held, including during preflight before a receipt exists. A value of
`null` with `boat_error` means lock inspection is unavailable or unsafe.
`boat_leases` lists held leases and slots with an unfinished record, with
`slot`, `held`, and `pid` when the kernel identifies the lock owner. `holders`
includes each unfinished Boat record with its `boat_slot`. The existing
`boat_lease_pid` is present when exactly one held lease has a known owner.
Listing observes the locks without creating or claiming them. A held lease, an
observed lease PID, an
unfinished receipt or unknown cleanup cannot be treated as idle. A stopped
receipt alone does not establish that the lease is free. Listing fails rather
than omitting a configured required local lock or an unreadable queue lock;
an absent optional extra local slot is allowed.
Queue/grant/exit records carry `run_key` (`<queued_ns>:<pid>`) for correlation
with a delegated helper. Its internal `ROTA_SLOT_RUN_KEY` handoff is removed
before the payload runs, like the queue-age/class metadata. Helpers that log
that key let a shared reader deduplicate ordinary native/helper exits while
retaining native cancellation/error exits when a helper never writes one.
Delegated legacy queue tickets record the helper's process start tick so
listing preserves their inherited FIFO age while still rejecting PID reuse.
Queue/grant/exit records also include `where` and local
fallback reasons. Complete remote logs and schema-1 receipts live under
`slots/runs/<id>/` beside the configured `state_file`; they are private and
not printed by listing. `boat.log` there records Boat's side of the run, one
timestamped and escaped line per event: each attach attempt and its exit code,
every non-output frame (heartbeats, error frames, exit frames), the Boat CLI's
own stderr, each status read, why the run gave up (`give-up <reason>`), and
the kill, copy, stop, usage and delete steps. It is capped at 2 MiB. The Boat
CLI's stderr during an attach also reaches the terminal, with control
characters escaped as `\xNN` so it cannot drive the terminal. A cleanup failure leaves a visible record that must
be reconciled before another remote VM starts.
An unreadable or incompatible Boat journal appears as `boat_error` in JSON
listing and a reason in text; local holders and waiters remain visible.
Usage dollars are Boat's metered list cost consumed from the included allowance,
not authorization to spend beyond it.
Only a verified trial or active standard `box_20` plan is admitted; wallet,
packs and nonstandard paid plans cannot bypass the included-credit guard.
Unknown identity/schema and near-reserve credit choose local before execution;
after execution may have begun, budget loss/provider stop is unresolved125,
with no local retry. Each start reads fresh limits. Cleanup always runs.
Private records retain `kill_sent`, monotonic `duration_seconds` (the run's
time up to its result, without teardown or later reconciliation), VM `usage`,
and observed `trial_seconds_used` with `trial_usage_reason`. Trial counter
deltas are account-wide observations, not exact per-VM metering. Unknown/tier
resets remain null. `queue_wait_avoided_seconds` is null with
`queue_wait_reason = "deferred-to-child26"`; no saved-wait estimate is claimed.

For a shared local fleet, `slots.lock_files` supplies one lock per configured
slot and optionally the timed extra slot; `legacy_dir` reads the existing
queue/quiet policy. Omit the extra lock to disable extra-slot grants in this
caller. Every fallback must use the fleet's same lock files. Do not acquire
an outer heavy lock around a remote slot invocation.
An absent `quiet-until` means no quiet hold. An existing malformed or
unreadable value refuses native listing/admission, including quiet-exempt
admission; it never becomes an implicit zero hold. The value is an unsigned
epoch timestamp in seconds. Quiet exemption bypasses a valid time hold only.

## Runtime identity and protocol bounds

`generation`, `created_build`, `active_build` and known `origin_build` use the
fixed `g1-` plus 32 lowercase hexadecimal spelling. They describe lifetime and
build roles, not configurable names. This baseline uses EONW v45; the ordinary
frame ceiling remains 2 MiB. Lifecycle `terminal_inventory` follows the existing
terminal-ID list and has at most 256 entries, with terminal/run IDs bounded by
128 bytes and at most 78,850 encoded inventory bytes. Unknown recovered launch
facts are null. These fixed format/resource bounds and machine field names
keep producers and consumers in agreement; no preference changes them.
`updating` is a fixed protocol End reason (exit 1), not a configurable timeout
or an ended lifetime. In-place Update remains planned.

Full Eon's private generation-directory `identity` file uses format 1: `EONI`,
little-endian u16 version 1, three raw 35-byte identities in generation,
creation-build and active-build order, then a 0/1 launcher-presence byte.
Presence 0 ends at 112 bytes. Presence 1 adds a little-endian u16 length and
1..4096 raw Unix bytes for an absolute path without NUL (114+N bytes,
4210 maximum; reads are bounded to 4211). Non-UTF-8 paths are preserved.
The directory is owned mode 0700 and the regular non-symlink file is owned
mode 0600. Publication writes and syncs a private temporary file, renames it,
then syncs the directory under the lifecycle lock. These fixed trust/format
rules are not preferences. eonbase creates no identity record.

## Built-in defaults

`eon config schema --json` describes this build's active configuration keys,
types, defaults (null when unset), bounds, choices, descriptions and when changes
take effect. It also identifies Eon-owned and required built-in popup fields;
`popups.<id>.*` rows describe custom popups only. It reads no user configuration
and works with a broken `config.toml`. Paths are unique; row order is unspecified.
Consumers allow extra fields and new values. `rules` is human-readable guidance;
use Runtime's configuration checking for validation, rather than parsing rules.

`eon config defaults` prints this build's defaults as commented TOML;
`eon config defaults --json` prints the same configurable values as a nested
object. Neither reads your configuration or creates directories, so both work
even when `config.toml` is broken. Unset settings remain comments in TOML and
are omitted in JSON. Eon-owned popup settings are omitted as editable keys.
The following block is checked against the command by the test suite.

<!-- defaults:start -->
```toml
# Built-in defaults. Unset and Eon-owned settings remain omitted.

[bar]
cpu_average_seconds = 30 # Seconds of CPU use that the hover's average covers.
cpu_critical_percent = 90 # CPU use from which the CPU chip turns red.
cpu_notice_percent = 60 # CPU use from which the CPU chip turns yellow.
cpu_temperature_critical_celsius = 95 # CPU temperature in Celsius from which the CPU chip turns red (Linux CPU sensors only).
cpu_temperature_warning_celsius = 85 # CPU temperature in Celsius from which the CPU chip turns orange (Linux CPU sensors only).
cpu_warning_percent = 75 # CPU use from which the CPU chip turns orange.
disk_critical_percent = 90 # Disk use from which the disk chip turns red.
disk_notice_percent = 60 # Disk use from which the disk chip turns yellow.
disk_path = "~" # Path whose filesystem the disk chip measures.
disk_warning_percent = 75 # Disk use from which the disk chip turns orange.
gpu_interval_seconds = 30 # Minimum seconds between NVIDIA GPU queries; a larger readable kernel autosuspend delay extends it.
ram_critical_percent = 90 # RAM use from which the RAM chip turns red.
ram_notice_percent = 60 # RAM use from which the RAM chip turns yellow.
ram_warning_percent = 75 # RAM use from which the RAM chip turns orange.
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "claude_quota_scoped", "cpu", "ram", "disk", "clock", "version"] # Eon Bar widgets: header widgets in display order; the clock sits at the bottom bar's left end and the version at its right end, wherever they are listed. Each quota widget selects a window; claude_quota_scoped selects all model-scoped Claude windows. Exact google_quota:<scope>:5h or :7d selectors are opt-in; Google windows show chips when reported.

[terminal]
background_blur = true # Request compositor background blur.
background_opacity = 0.8 # Opacity of the terminal's default background.
# border_width: unset. Use Venus's neutral outline width when omitted.
# columns: unset. Let the window choose initial columns when omitted.
# cursor_trail_color: unset. Omission retains the session's random cursor colour.
cursor_trail_duration = 1.5 # Cursor tail duration multiplier; 0 turns the trail off.
detect_local_hosts = true # Treat plain-text localhost, 127.0.0.1 and [::1] with a port as links.
# focus_border_width: unset. Use Venus's focus outline width when omitted.
# focus_color: unset. Use Venus's focus colour when omitted.
font_fallbacks = ["Symbols Nerd Font Mono", "Accelerando Terminal"] # Ordered fallback font families.
# font_family: unset. Let Venus choose the font family when omitted.
# font_size: unset. Let Venus choose the font size when omitted.
# line_height: unset. Let Venus choose the line height when omitted.
padding_color = "extend" # How terminal padding takes its background colour.
pane_frames = true # Draw rounded pane frames.
# rows: unset. Let the window choose initial rows when omitted.

[panels]
heading = "connected" # Panel heading position: notch or connected. Applies on open, reopen or reload.

[shell]
atuin = true # Enable the managed Atuin history integration.
carapace = true # Enable the managed Carapace completion integration.
command = ["eon-nu"] # Direct argv for terminals without an explicit command.
mise = true # Activate host mise when available and not already active.
starship = true # Enable the managed Starship prompt integration.
zoxide = true # Enable the managed Zoxide integration and record tab-directory picks.

[anima]
duration_seconds = 3 # Startup animation duration in seconds.
enabled = true # Enable this feature or popup.
style = "random" # Startup animation style.

[session]
animate_working_badge = true # Animate Working and Background badges; false keeps both still.
number_keys = true # Alt+Shift+1..9 and Alt+Shift+0 show sessions 1..10; false leaves the keys to the terminal.
warning_close_last_tab = true # Ask before stopping the final tab's terminals.

[keys]
input_lock = "Alt+Shift+I" # Toggle terminal input pass-through for this window.
release = [] # Pass these fixed physical shortcuts to the focused terminal.

[sidebar]
enabled = true # Enable this feature or popup.

[attention]
notify = true # Send desktop agent attention notifications on Linux.
notify_names = true # Use location names in agent attention notices. False uses IDs.

[agents]
send_enter_delay_ms = 300 # Wait between `eon pane send` text and its --enter.
send_typing_quiet_ms = 2000 # Refuse unforced sends within this many ms of typing; 0 disables (0 to 60000).
terminal_access = "window" # "none" denies pane read/history/send and CLI Scrollback, even when you run them yourself; the window shortcut still works. This is not sandbox isolation.

[editor]
command = ["hx"] # Direct argv that `eon edit` and Files run with the file appended.
kind = "auto" # Recognize packaged Helix or nvim; "neovim" opts a wrapper in.
open_in = "popup" # Open in the Helix or Neovim Editor popup, or "pane" for a new pane.

# [updates]
# nix_profile: unset. Absolute profile path; set with nix_entry.
# nix_entry: unset. Exact profile entry name; set with nix_profile.
# bundle_prefix: unset. Absolute existing managed prefix; excludes the Nix pair.
# allowed_signers: unset. Absolute independently trusted OpenSSH signer file.

[popups.agent]
command = "auto" # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+L" # Popup shortcut using physical keys.
label = "Agent" # Popup display name.

[popups.anima]
# command and keep_alive are Eon-owned.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+A" # Popup shortcut using physical keys.
label = "Anima" # Popup display name.

[popups.commands]
command = ["eon-commands"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = false # Keep this popup running when hidden.
keybinding = "Alt+Shift+M" # Popup shortcut using physical keys.
label = "Commands" # Popup display name.

[popups.editor]
# Editor uses [editor] command and keeps running when hidden.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+O" # Popup shortcut using physical keys.
label = "Editor" # Popup display name.

[popups.files]
# Files is required: it is the tab's directory pair.
keybinding = "Alt+Shift+Y" # Popup shortcut using physical keys.
label = "Files" # Popup display name.

[popups.git]
command = ["eon-lazygit"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = true # Keep this popup running when hidden.
keybinding = "Alt+Shift+J" # Popup shortcut using physical keys.
label = "Git" # Popup display name.

[popups.project]
# Project is required and shows the tab's Files on quick search.
keybinding = "Alt+Z" # Popup shortcut using physical keys.
label = "Quick search" # Popup display name.

[popups.scrollback]
# command and keep_alive are Eon-owned.
enabled = true # Enable this feature or popup.
keybinding = "Alt+Shift+E" # Popup shortcut using physical keys.
label = "Scrollback" # Popup display name.

[popups.sessions]
command = ["eon-sessions"] # Direct argv, or auto for Agent's command selection.
enabled = true # Enable this feature or popup.
keep_alive = false # Keep this popup running when hidden.
keybinding = "Alt+Shift+S" # Popup shortcut using physical keys.
label = "Sessions" # Popup display name.
```
<!-- defaults:end -->

## Configuration behavior

Each build ignores keys it does not know, with one warning per key and read
naming its dotted path and the configuration file. This lets an older running
window keep using the settings it knows after a newer build adds a key. Known
keys with wrong types or values still fail, including unsupported widget names.

A key that a later release deprecated still loads, read as its replacement,
with one warning that names the replacement; Eon never rewrites
`config.toml`. When the replacement is set too, it wins and the deprecated key
is ignored with a warning. `[bar] show_version`, deprecated in v0.1.2, reads as
`[bar] widgets`: `false` gives the built-in widget list without `version`, and
`true` gives the built-in list. Any other value is a configuration error.

A table that a later release renamed has no effect: Eon ignores it, whatever
it holds, with one warning that names the new table, and never fails to start
over it. v0.1.20 renamed `[workspace]` to `[session]` and
`[popups.workspaces]` to `[popups.sessions]`; move their keys by hand.

`[bar] widgets` chooses which Eon Bar widgets show and their order, left to
right between the tabs and the Shortcuts button, except `clock` and `version`,
which sit at the left and right ends of the bottom shortcut hints while they
show, wherever they are in the list: the Codex and Claude quota chips, CPU, RAM,
disk and NVIDIA GPU use, the local clock and the version label (`EON α0.1.24`,
`EON β0.2.0`, or `EON 1.0.0`). The release stage is a fixed build identity,
not a preference; omit `version` to hide the entire label. The Eon logo at the
bar's left end is not a widget and shows whatever the list holds. Each name
appears at most once. A widget left out doesn't show, `widgets = []` leaves
only the logo, the tabs and the buttons, and omitting the key uses the built-in
list above. The list applies when a window next opens, reopens or reloads.
It also accepts opt-in exact Google scope/window selectors for Google chips.
See [Agent quota](#agent-quota) for grammar and examples.
Unknown names are ignored with a warning; repeated known names are a
configuration error. Narrow windows hide GPU, disk, RAM and CPU, whatever the order,
then quota chips one by one, the one closest to the tabs first. Quota chips
yield before the session label and Eon icon, preserving room for the Window
menu, the tabs' minimum area and trailing controls. Tabs can overflow and
scroll beside the icon; only a tiny width that cannot fit the icon with those
minima hides it. Only selected quota chips are measured, using their displayed
text; hidden windows reserve no room. Changing values or window positions can
change which whole chips fit. In the shortcut hints the version hides first
and then the clock once the keycaps no longer fit beside them. The version
belongs to its running supervisor; updating the installation does not change it.

Header widgets use the unselected tab's filled capsule with a neutral outline
and a small status dot at the right. The session label uses the same fill without
a resting outline. CPU, RAM, disk and GPU percentages share the interface font
with tab labels and quota chips; each reserves its widest value through 99%.
Only 100% widens a chip by the extra digit and reflows it and the chips to its
left. Each icon keeps its left inset. The percentage aligns before the dot's
reserved space, with at least 2 logical pixels between icon and percentage.
The Shortcuts button is a plain `?` with the other bar controls' hover and mint
keyboard-focus treatment. Clock and version keep their bottom-corner monospace
text and cyan colour.

The fixed chrome recipe uses a dot diameter of half the text padding and a gap
of one third: 6 and 4 logical pixels at default typography. Both round to whole
physical pixels; the dot's origin snaps to pixels and centres on the text ink.
Every chip reserves that space even when unjudged quota has no dot. The outline
is 1 logical pixel, rounded to whole physical pixels with a minimum of 1.
These sizes are fixed tokens: one shared recipe keeps supported chip layouts
bounded and the bar calm, without a separate style setting.
Chip text stays neutral; mint remains keyboard focus, which reading chips cannot
take. There are no progress meters.

Normal readings have a neutral dot. Quota dots start at yellow when projected
shortfall reaches 5% of the window, then blend through orange at 10% and red
at 25%; only at 25% or above does the outline also turn red. Pace judgement
waits for minimum window history (at least `min(duration / 10, 4h)` elapsed:
30m for 5h, 4h for 7d, and at least 5% used, or since a mid-window reset move
beyond `min(duration / 20, 5m)` observed in memory by the running window);
these noise-floor bounds stay fixed alongside the pace anchors. An exhausted
fresh window with a reset (0%
remaining) is always judged and keeps at least a yellow dot. Stale, reset-less,
blocked, unknown, and below-minimum-history quota has no dot. Healthy judged
quota keeps a neutral dot and outline. Hover and accessibility describe pace in
words; the numeric label reports remaining amount, not the pace grade.

CPU, RAM and disk dots turn yellow from 60% use, orange from 75% and red from
90% by default; the outline turns red only at Critical. For each chip,
`<chip>_notice_percent`, `<chip>_warning_percent` and `<chip>_critical_percent`
set those levels (`cpu`, `ram` or `disk`). CPU compares the 2-second use it
shows. On Linux with a supported CPU sensor, its dot also turns orange from
`cpu_temperature_warning_celsius` (85 °C) and red from
`cpu_temperature_critical_celsius` (95 °C); the stronger grade wins, including
the critical outline. Both temperature levels accept whole degrees from 30
through 120, with critical not below warning; equal levels skip orange. macOS
has no CPU temperature reading. Hover and accessibility say in words why a
chip is graded, and give CPU's average over the last `cpu_average_seconds`.
Use levels compare the whole percentage the chip shows; temperature levels
compare the whole degrees in its hover. Percentages accept whole numbers from
1 through 100, with notice not above warning and warning not above critical
for each chip; equal levels skip the lower colour. Omitted percentage levels
start at their defaults and adjust only as needed around explicit levels: an
omitted warning is raised to an explicit notice or lowered to an explicit
critical when needed, an omitted notice is the lower of its default and the
resolved warning, and an omitted critical is the higher of its default and the
resolved warning. Explicitly inverted levels fail. For example,
`cpu_warning_percent = 95` alone resolves to 60/95/95; `ram_warning_percent = 50`
with `ram_critical_percent = 85` resolves to 50/50/85. `cpu_average_seconds`
accepts 2 through 600. The defaults are listed above. Like `widgets`, they
apply when a window next opens, reopens or reloads, and an out-of-range value is a
configuration error.

CPU hover also names the CPU, gives its temperature when available, the
busiest logical CPU over the same 2 seconds (numbered from zero), and the
1-minute load average to one decimal. Linux temperature comes only from
coretemp’s Package id 0 or k10temp’s Tctl (Tdie when Tctl is unavailable),
never from a board or ACPI zone. Missing details are omitted.
See [Eon Bar](../venus/docs/PRESENTATION.md#eon-bar).

The opt-in GPU chip (`gpu`, add it to your `[bar] widgets` list) shows the busiest NVIDIA
GPU's utilization on Linux. Its expansion-card symbol is fixed by the bundled
Symbols font. Hover and accessibility list every discovered GPU by its measured
name and PCI address, with each reading or unavailable/unsupported status.
A sleeping NVIDIA GPU shows `0%` with `asleep, not queried`; its actual sampled
percentage is absent. Missing tools, permissions or failed readings contribute
no value. Intel and AMD utilization are unsupported in this reader.

`gpu_interval_seconds` (default 30; whole seconds from 5 through 3600) sets the
minimum time between NVIDIA queries across every NVIDIA device and Eon window,
measured from completion, including failed queries. Devices are queried in turn,
one per interval; hover retains their most recent readings. The 30-second default is provisional: early power-state observations
showed active-to-suspended windows of about 10.3 seconds, but the delay after
other GPU work ends has not yet been isolated. Live sleep/default validation
remains pending while other applications keep the GPU active. GPU is excluded
from the default list until that proof is available (`eon-9wewk`). The largest
readable kernel autosuspend delay among NVIDIA devices extends the interval by
a one-second margin. The host driver's `nvidia-smi` must be available on PATH. Queries are
PCI-specific, bounded to one second and 4 KiB output, and coordinated across
Venus windows through a private runtime cache. There are no immediate retries. Occluded windows stop scheduling GPU reads;
becoming visible refreshes immediately subject to the same shared interval.
Only a currently runtime-active GPU is queried; a sleeping or unknown-state
NVIDIA peer suppresses vendor queries too. A GPU that suspends in the gap
between the check and query can wake for an extra idle delay. These settings
apply on open, reopen or reload. GPU colour grades are not used: utilization
alone is not evidence of thermal or memory pressure.

GPU readings are not yet in `eon session --json`; shared CPU/RAM/disk/GPU data
parity is tracked by `eon-agent-parity-a9g0.12`, without extra GPU queries.

The disk chip shows how full the filesystem holding `disk_path` is, read every
30 seconds: used space over used plus available space, rounded up as GNU df
rounds it. `disk_path` is an absolute path, or `~` or a path starting with
`~/` for your home directory and paths under it; empty, relative and `~user`
paths are configuration errors. A path that doesn't exist is not an error: the
chip shows no reading. The chip uses the same notice, warning and critical
grades described above. The 30-second
refresh is fixed, since disk use changes slowly. Like the other `[bar]` keys,
they apply when a window next opens, reopens or reloads.

Omitting `cursor_trail_color` chooses a random color that lasts as long as the
running window, so `eon window attach` reopens with the same color; a new
window picks again. eonbase keeps one color across reopens, though its
first window may differ. Explicit
values are `random`, `preset:<name>`, or `custom:#RRGGBB`. The presets are
`magma`, `solar`, `lime`, `forest`, `ice`, `ocean`, `nebula`, and `bubblegum`;
the terminal adds a contrasting cursor outline automatically. The cursor body
uses the chosen color with a shape-aware contrasting edge unless Orbit supplies
an explicit cursor color.

`focus_color` colours the selected pane's whole outline in the selected column
when pane frames are on, the current Agents row's outline, and the working and
background dots of agent attention badges on tabs, in the agent sidebar and
in the attention inbox. Each attention glyph starts in its state colour;
working and background dots start in `focus_color`. Every glyph adapts only
as far as needed for 3:1 against its surface, before pulse opacity and stale
dimming. Stale glyphs and outlines apply a 0.45 dimming multiplier after pulse
opacity; either multiplier may reduce instantaneous contrast below 3:1.
Badges draw no backing at any opacity. Bar corner badges retain contrast
against both their pill and the surrounding bar; overflow badges use the
visible tab fills beneath them. Under a translucent terminal background
(`background_opacity` below 1, default 0.8), an unhighlighted row's surface
depends on the desktop behind the window. Its glyph adapts for 3:1 against
the card colour composited at that opacity over both black and white. Only
where no single colour reaches 3:1 against both does it retain its source
colour and take a black or white outline, whichever contrasts more with the
glyph. This contour follows the glyph itself, about one physical pixel wide,
reduced only to preserve gaps in small glyphs. Sidebar/inbox app marks use
1.19 line heights, capped at 1.6 times the font size
before snapping to the nearest multiple of Clawd's eleven columns (11–253
physical pixels). Default boxes are 22px at 1x and 1.25x, 33px at 1.5x and
44px at 2x. Status glyphs use rounded font-sized boxes (16/20/24/32px at those
scales), capped at 256px. Each centres independently across the full row;
a half-cell gap separates symbols and text. Tab and
bar badge sizes and the popup marker retain their own
metrics. These sizes and the automatic contrast/outline recipe stay fixed to
bound layout and readability variants; font size and opacity remain
configurable (`eon-0ntg1`, `eon-sidebar-app-icons-y99b7.11`). `focus_color` accepts
`preset:<name>` from the cursor presets above or `custom:#RRGGBB`; `random` is
invalid. The setting is omitted by default, leaving Venus's built-in focus
colour in use. The example selects `custom:#b207fb`. F6 keyboard focus colours a framed pane title's edge span mint;
other chrome controls keep their mint pill outline. This setting applies to full-Eon surfaces;
eonbase has no pane outline.

`border_width` sizes neutral pane-card, agent-sidebar-card, popup and native dialog outlines,
including their title edges and the popup marker in Agents and the attention inbox.
Eon Bar widgets use their fixed status-outline recipe; `border_width`
does not affect them. `focus_border_width` sizes the selected pane's
whole outline. Both accept integers from 1 through 3 logical pixels; defaults
are omitted, leaving Venus's built-in widths in use. With the example's widths,
physical widths are normally `max(1, round(width * scale))`:
at scale 1.3 they draw at 1 and 1 physical pixels, and at scale 2 at 2
and 2. The popup marker alone reduces a stroke that would fill its smaller
squares, keeping both hollow: at font size 6, scale 1 and `border_width = 3`,
the marker uses 2 physical pixels. Pane frames off hides pane and sidebar outlines and title rules;
popups keep their outline. These settings apply to full-Eon surfaces;
eonbase receives neither override. F6 title focus uses the pane header's configured
stroke; other focus pills keep their one-physical-pixel outlines. The current
agent row has a one-physical-pixel outline in `focus_color`, adapted only as
needed for 3:1 against the sidebar background, without a selection fill. Hover
or keyboard focus adds the dark highlight fill to any row, adapted only as
needed for 1.5:1 against the background; the current outline keeps its colour.
Its mint keyboard-focus outline is one physical pixel, inset one pixel on a
current row and adapted only as needed for 3:1 against the row fill. Both row
strokes stay one physical pixel to match keyboard focus and the approved look;
pane/card border settings do not size them. Current text retains 4.5:1 against
the background or highlight fill. Selected tabs keep their fill.
Resting numbers and IDs are dimmer than names where 4.5:1 remains possible;
the selected tab label uses one primary text role. These colours follow the
fixed chrome palette; colour settings belong to the themes work.

`cursor_trail_duration` defaults to `1.5`. Set it to `0` (or `0.0`) to turn
the trail off while keeping the cursor's color and contrasting edge. To enable
the trail, use a finite multiplier from `0.25` through `4.0`; larger values
keep it visible longer. It applies when an Eon or eonbase window next opens or
reopens. Eon owns this setting for both products; no Venus or Orbit
configuration is needed.

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
and Close (`×`). The zoomed tab also shows `▣` after its name. Zoom/Restore
glyphs are fixed together so the tab and button keep the same meaning. The expanded pane's framed header keeps both in its band and a
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
Action size, spacing and header corners derive from the configured typography
and existing title padding. Border widths, focus color and background opacity
apply to the continuous band. There are no separate action-geometry settings;
the controls keep full targets and text/curve clearance across supported fonts and scales.
The Hide buttons on popup and Agents headings and the Close button on each
Stack tab follow the same rule: their size, glyph and labels are fixed and
derive from the configured typography and title padding.

`[panels] heading` selects the heading position for Stacks, popups, Agents and
native dialogs. `"notch"` keeps the double notches described below.
`"connected"` (the default) joins them with a straight top edge at notch height
and the card's usual rounded outer corners, enclosing the existing title and
actions without a heading fill or separator. Titles, action targets, terminal
rows and pane header bands keep their positions. Tight headings retain the same
compact fallback.
The raised top outline stays neutral at `border_width`; the selected pane's
focus colour and width begin below the existing card-corner join. With pane
frames off, Stack and Agents labels remain plain; popup and dialog frames follow
the selected position. The setting applies when a window opens, reopens or
reloads with `eon window reload ID`.

Popup and Agents headings have one Hide button (`–`) at the right, with the
name to its left. In the notch position, the button sits in a second notch
mirroring the title's. The popup's
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

Each Stack tab has one Close button (`×`); in the notch position, it sits in
the right notch. Close always asks before it closes anything;
[Usage](USAGE.md) describes its prompt.

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

The pane band and the dimensions of both panel heading positions follow the configured font
size and line height (defaults: 16 px and 1.125), existing title padding, and `border_width`
(default: 1). A selected pane retains its `focus_border_width` and focus colour.
Terminal-backed fills use `background_opacity` (default: 0.8). The fixed policy
and its support-cost rationale are recorded in `eon-27deg`, `eon-2ibdd`,
`eon-u5uv2`, `eon-xaq6c` and `eon-yjrru`. The gap above a band follows the
title row height. Deriving both from the same typography keeps their spacing
in step
and gives short windows a single clearance rule. The Stack tabs are
fixed the same way: their labels (`Stack`, `Stack N`), the stack band (one title
row plus a third of the title padding: 24 px at the defaults) and the card gap
between the bar's pills and the tab tops (default: 8 px) derive from the font
size, line height, title padding and card gap. Their position is selected by
`[panels] heading`; their dimensions stay derived from these shared metrics
(`eon-whjhd`, `eon-6ui6p`).

`padding_color` controls terminal padding in panes, popups and eonbase.
`extend` uses the nearest edge cell's background on each side and
at the corners. On the primary screen, top or bottom padding keeps the default
background if that edge row contains any default-background cell (including an
explicit color matching the default) or a full-height powerline glyph.
Alternate-screen TUIs always extend. `extend-always` skips those exceptions;
`background` keeps the terminal default background throughout the padding.
Default backgrounds retain the configured opacity; explicit backgrounds paint
as they do in cells. Padding stays inside the rounded card and below outlines.
Terminal padding (around the grid, popup margins and frameless gutters) follows
`padding_color`; it never reaches above the terminal's top edge into header
chrome. Pane header bands keep the shared column background and opacity,
including lone and zoomed panes. The setting applies when a window opens or
reopens.

`background_opacity` sets how opaque each terminal's default background is, in
Eon and eonbase alike. It also covers the terminal padding, the title rows and
gaps that share that background (above), and the agent sidebar card. Hovered and
keyboard-focused agent rows stay opaque. Explicit cell backgrounds, selection,
text and the cursor keep their own colours, and the Eon Bar stays the opaque
bar colour. With pane frames on, so do the gaps and footer outside the cards;
with them off, the frameless terminal-background
strips below the bar, sidebar and footer included, take this opacity.
See-through areas still take clicks. `background_blur` asks the compositor to
blur what shows through. It is independent of opacity, so a fully opaque
background hides it, and a compositor without blur ignores it. See [Window
background and
decorations](../venus/docs/PRESENTATION.md#window-background-and-decorations).

`detect_local_hosts`, in Eon and eonbase alike, makes plain-text
`localhost:3000`, `127.0.0.1:8080` and `[::1]:5173`, with an optional path, a
link like a printed `http://` URL: Ctrl+click opens and Ctrl+Shift+C copies
`http://` followed by the text. Set it to `false` to link only URLs with a
scheme. See [Hyperlinks](../venus/docs/INTERACTION.md#hyperlinks).

Opacity accepts `0.0` through `1.0`. Blur and pane frames are booleans. Font size
accepts 6–96 logical pixels, line height accepts 1–3, and a supplied columns and
rows pair must fit 100,000 cells. `font_fallbacks` accepts an ordered array of at
most eight installed family names. The built-in order is listed above, so alien
glyphs avoid searching all installed fonts.
An explicit list replaces that order. The terminal package includes fonts for
ordinary text, Nerd Font icons, Accelerando and, on Linux, Noto Color Emoji; it
does not install them system-wide.
Compositors may ignore blur or initial window
sizing.

The terminal settings apply when a surface opens or reopens. Shell settings are
read for each terminal. Invalid startup configuration creates no fresh terminal;
a failed replacement leaves existing terminals and commands alive.
In full Eon, [`eon window reload ID`](USAGE.md#reload-window-configuration)
rereads presentation settings by replacing Venus while terminals keep running.

On a fresh full-Eon window, Anima plays after the desktop is ready and before
the first tab's quick search. Press any key other than its previous/next style
keys to dismiss it immediately. `enabled = false` skips startup playback, and
`eon --no-anima` (also on `eon run` and `eon window new`) skips it for one
launch; `style` is passed to the pinned Anima executable, and
`duration_seconds` accepts 1–30. Anima owns its style names; run
`eon anima --help` to see them. A failed animation continues to quick search.
Attachment, recovery, later tabs, eonbase, and a launch with `--dir` do not play
the startup animation.

Closing the last tab quits its window and stops its terminals. While the
window holds other sessions, it closes only that session.
`warning_close_last_tab = true` makes `eon tab close` and
`eon pane close` ask first when run from a terminal, listing the terminals that
will stop and, while other sessions remain, naming the session that
closes; `--json` never asks. In the window, Alt+Shift+W, the bar's `×` and
Alt+W on the last pane show a prompt listing the same terminals: Enter quits,
or closes the session while others remain, Escape cancels, and Alt+Shift+D
keeps them running instead. With `false`, both close an idle final tab at
once. The CLI's busy confirmation for `eon tab close` and `eon pane close` does
not depend on this setting: when a terminal the close stops is busy or its busy
state is unknown, it still asks, naming the final consequence, or with `--json`
requires `--yes` ([CLI commands](USAGE.md#cli-commands)). It stays fixed
because stopping busy work needs explicit consent. The window reads the setting
when it starts.

A Stack close always asks, whatever this setting is: `eon column close` needs
`--yes`, and a Stack tab's Close button shows its prompt even for one idle
pane. The prompt has no off switch, because its answer is the consent that
binds the close to the exact terminals listed. Enter in that prompt is
ignored for 400 ms after the list first appears; this confirmation delay is
fixed, a safety floor that keeps a key pressed before the list could be read
from approving it.

`animate_working_badge = true` animates Working and Background attention badges
on tabs, bar corners, sidebar and inbox rows. Working steps through the braille
dots ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ten times a second. Background uses three round dots with a
slower staggered pulse, repeating every three seconds between 0.65 and 1.0
opacity. Idle uses a small muted filled dot (5.6px across in its default 16px
box); Unavailable uses a hollow round ring. Idle's appearance is fixed by
Lucca's explicit choice (2026-11-14: “i dont want an extra seting”). Their
edges use one-physical-pixel anti-aliased coverage.

`false` keeps Working at ⠋ and Background's round dots steadily lit, for people
who prefer no motion. A software (CPU) graphics adapter also keeps both still.
The existing key retains its name and governs both attention animations; there
is no second pulse switch. Cadence and pulse range stay fixed to bound wake cost
and readability variants. The window reads the setting when it starts.

`number_keys = true` makes Alt+Shift+1 through Alt+Shift+9 show the window's
first through ninth session, in `eon session ls` order, and Alt+Shift+0 the
tenth. `false` leaves those keys to the program in the terminal, for example
Emacs's M-% on Alt+Shift+5. A popup configured on one of them takes only that
key either way; with `true`, Alt+/ lists that key as overridden. The window reads the setting when it starts.
See [Keybinding clashes](USAGE.md#keybinding-clashes).

### Keyboard shortcuts

`[keys] input_lock` sets the window's input-lock toggle. The default is
`"Alt+Shift+I"`, shown as Option+Shift+I on macOS. Use the same modified
physical-key spelling as [popup bindings](#popups). For example:

```toml
[keys]
input_lock = "Alt+Shift+P"
```

Choose a chord that is free on both platforms. Fixed Eon bindings, the
bundled macOS Hide and Hide Others keys (`Super+H` and `Alt+Super+H`), and
effective bindings of enabled popups conflict with the toggle. A conflict
fails configuration validation and names the setting and conflicting chord.
Start a fresh window to apply a changed toggle. Reopening or reloading the
presentation keeps the supervisor's accepted toggle and lock state.
See [Input lock](USAGE.md#input-lock).

`[keys] release` passes individual fixed shortcuts to the focused terminal.
The default is `[]`. Use any combination of `"Alt+M"`, `"Alt+W"`,
`"Alt+Q"` and `"Alt+G"`, with that exact spelling on Linux and macOS.
For example, `release = ["Alt+M", "Alt+W"]` releases pane creation and pane
close. Invalid values, repeated chords, and chords also bound to an enabled
popup fail configuration validation. The next supervisor start applies the
setting. Reopening or reloading a window with its existing supervisor keeps
that window's applied releases.

`[sidebar] enabled = true` offers the agent sidebar, which lists
the coding agents in the window's tabs that report their status. `false` turns
it off. With their bindings active, Alt+G and Alt+Shift+G still jump between
agents. The window reads the
setting when it starts. Alt+Shift+H shows or hides the sidebar; it also steps
aside while the window leaves fewer than 80 cells for the terminals beside it.
See [Agent sidebar](USAGE.md#agent-sidebar).

### Agent attention notifications

On Linux, full Eon sends a desktop notification once when an agent fails,
needs input or finishes outside the pane or popup you're viewing. A finish
held by background work can notify when the hold clears.
`[attention] notify = false` turns off these notices and their sounds.
`notify_names = false` shows session, tab and pane or popup IDs. The default
uses the session name, the tab's launch-directory name, and the pinned pane
name or configured popup label, with IDs where names are unavailable.
The window reads both settings when its supervisor starts.

Error and needs-input notices request the desktop theme's notification
sound. Finished notices request silence. COSMIC and mako currently display
these notices without playing the requested sound (`eon-mqmli` tracks COSMIC
playback). When the notification service is absent, notices resume on its
return or after the next successful detach notice. When the bus or worker
fails, a later detach lets Eon reconnect. Missed notices do not repeat.

### Agent terminal access

`[agents] terminal_access` limits the direct terminal-content commands used
by well-behaved agents through Eon's workspace control socket. It applies
with or without a calling terminal, including commands run outside Eon.

- `"window"`, the default, lets any process running as you read or send to
  any pane or popup of the window, in any session, shown or hidden.
- `"none"` refuses `eon pane read`, `eon pane history` (including `--edit`)
  and `eon pane send` with `terminal-access-off`. It also refuses
  `eon popup show scrollback`, `eon focus uN` for an existing Scrollback,
  and Scrollback requests from other programs even when they name no
  calling terminal. These requests return no screen or history text and
  fail before reading history, creating a snapshot file or starting its
  editor. Explicit `eon popup hide uN` remains available. These commands are
  refused even when you run them
  yourself in a shell pane; use the configured Scrollback shortcut to open
  history in the window. The shortcut keeps working because its request
  comes from the supervised window process, rather than merely naming no
  calling terminal.

The window reads the setting when it starts. `none` is a direct-content
preference and kill switch. It does not isolate an agent or other client
that can reach Eon's socket. Workspace inspection still exposes metadata
such as terminal titles, working directories and reported attention. The
setting does not restrict host command creation
through `eon pane create` or `eon tab create`; command-bearing requests
require a live caller assertion, which supplies routing rather than
authenticated agent authority. Editor RPC and Quick-search helper sockets
have separate access rules and are not disabled by this setting.

Window reload is also separate. Even under `none`, a client with no calling
terminal can request a window reload while the window is focused, choosing
a canonical immutable executable in the Nix store with matching protocol
requirements. Its replacement receives the window process's access to
Scrollback and Orbit terminal attachment, bypassing the direct-command
restriction. Constraining this replacement authority is tracked in
`eon-260mh.2.5`. Programs permitted to inject keystrokes can also trigger
the window's actions; those requests are treated as native input too.
Sandboxed agents must not reach Eon's socket or auxiliary host sockets. See the
[security model](SECURITY-MODEL.md#workspace-control-and-terminal-access).

### Terminal orchestration

The default remains `"window"`. If you do not run terminal orchestration, we
recommend setting `[agents] terminal_access = "none"` in `config.toml` before
opening a new window. Running windows retain their startup value. See the
[public security policy](../SECURITY.md#agent-security-during-the-beta) for
the accepted beta risks and sandbox guidance.

`[agents] send_enter_delay_ms`, 300 by default, is how long
`eon pane send --enter` waits after the text before sending Enter, so agent
prompts take the whole paste first. It must be 0 to 5000.

`[agents] send_typing_quiet_ms`, 2000 by default, refuses each unforced CLI
paste or Enter while attached-client typing is queued or within this many
milliseconds of typing. It must be 0 to 60000; 0 disables the check.
`eon pane send --force` bypasses only this check. CLI sends never extend the
interval. Like the other agent settings, the window reads it when it starts.

Pane labels use a fixed policy: a pin, then sanitized terminal title, then compact
working directory, with differing-directory context. Commands and `eon pane rename`
set or clear a pin; pins are session state rather than configuration. The pane
ID dims only when the resolved fill and configured opacity permit readable contrast.

## Agent quota

Google selection is opt-in through exact `google_quota:<scope>:5h` and
`google_quota:<scope>:7d` entries in `[bar] widgets`. These selectors
enable Google collection and its CLI JSON facts. Each selected active Google
window shows a chip; unselected or missing windows show nothing. The ten
existing defaults above include no Google entries.

Google chips show the Gemini spark and `<position>/<duration> <remaining>%`, with
`old` when stale, the same shape as the Claude and Codex chips. Without a reset
they omit the elapsed position and slash. The `Gemini Models` group's chip
shows the bare spark; any other group's chip leads with its name's initial,
such as `C 2d15h/7d 0%` for `Claude and GPT models`. Selected groups that share
an initial are numbered in widget order (`G1`, `G2`), so every selected group's
chips stay apart. Hover and accessibility name Google, the model group, period,
remaining allowance, reset availability, local read time and freshness.
Disabled windows have no chip; weekly-only groups get no invented five-hour
chip. Hide Argon by leaving out `google_quota:Gemini 4 Argon:7d`; all its
active facts remain in `eon session --json` while any Google selector is
enabled.

Scope is the exact reported model-group name: nonempty, control-free and at
most 32 UTF-8 bytes. The last colon separates `5h` (five hours) or `7d`
(seven days); commas and earlier colons belong to scope. Case and whitespace
are preserved. For example, `"google_quota:A,B:C:7d"` selects scope `A,B:C`.
A source rename needs a configuration edit. There is no bare `google_quota`
widget or separate hide key. Unknown or malformed selectors warn and are ignored;
duplicate recognized entries and non-string/non-list values are errors.
Omitting `widgets` keeps the defaults; `widgets = []` selects none.

```toml
[bar]
widgets = ["google_quota:Gemini Models:5h", "google_quota:Gemini Models:7d", "clock", "version"]
```

`eon config schema --json` lists fixed widget names in `choices` and the two
parameterized Google forms in `element_forms`, including scope byte/control
bounds. Runtime owns validation; Venus reads only resolved launch arguments.

Google collection uses your already signed-in `agy` on PATH, versions 1.4.1
and 1.5.6. Only absolute PATH entries are searched; empty and relative entries
are skipped so an `agy` planted in the supervisor's starting directory is never
selected through them. Each scheduled attempt verifies `agy --version` before
running `agy -p /usage --output-format json` with that same resolved executable.
There is no inference fallback, bundled agy, login or account switching. A missing,
nonexecutable or unsupported agy records failure reason `start`; the last good
reading remains available. `eon doctor` says which: it names an unsupported
version and the supported ones. The Google subscription scope is Antigravity's
baseline model-group allowances, not Gemini Apps, Gemini API or AI-credit balances.
The intended support scope is consumer OAuth on x86_64 Linux native Wayland
with Nix; account-class and plan-tier verification remains open. The installed
observation did not verify either, and a successful usage report alone does not
establish them. API-key and
enterprise setups are outside that scope; Eon
preserves intentional provider settings rather than stripping them and does not
read credential files or rewrite agy's settings.

Any valid Google selector enables collection even if that group is unreported.
`eon session --json` carries every active reported Google window, including
unselected groups. Collection is checked on open, reopen and accepted reload;
removing every Google selector cancels the current read and clears memory and
failure, while keeping the last good disk reading. A read runs away from the
workspace in an empty private directory with null stdin, sanitized session
context, suppressed stderr and process-group cleanup. Its version check and
usage command together have one ten-second deadline and one 64 KiB stdout cap.

Google starts at least five minutes apart, including failures and re-enabling.
Supervisors share `google.schedule` beside the retained reading under a short
file lock, showing shared facts and failure while waiting; an unusable schedule
falls back to each supervisor's own spacing. The first provider error since
the last success waits five minutes plus up to one minute of jitter; further
provider errors wait 30 minutes, then one hour, then two hours until success
restores five minutes. Only a success resets the count, so a provider error
after another failure that follows a provider error takes the 30-minute
step. A supervisor started from an older Eon writes its own, longer delay,
which the others honor until it restarts. Successful readings atomically
replace versioned normalized
`$XDG_STATE_HOME/eon/quota/google.json` (`~/.local/state/eon/quota/google.json`
by default); valid empty reports clear it. Failures keep the last good reading.
Retained readings are available on restart, become stale after one hour and
expire after 24 hours; each supplied reset expires its own window. Missing or
disabled data is never zero usage. These cadence, resource, freshness and format
bounds are fixed request-cost, safety and support limits.

Eon bundles pinned CodexBar CLI for Codex and Claude quota.
Codex uses your authenticated `codex` executable on `PATH`; Claude uses its local
OAuth credentials. Explicit sources avoid browser fallback and model prompts.
Eon does not change provider settings or manage accounts. CodexBar owns credential
selection and its local authentication/cache mechanisms.

Codex and Claude show one chip per available window, five-hour then seven-day.
Claude's model-scoped weekly windows, such as `Fable`, follow as their own
chips. Choose chips with `codex_quota_5h`, `codex_quota_7d`,
`claude_quota_5h`, `claude_quota_7d` or `claude_quota_scoped` in `[bar] widgets`.
The duration names select unscoped windows only; `claude_quota_scoped` selects
all model-scoped Claude windows, in provider order. All five widgets are listed
by default. Codex's 5h widget also shows its blocked or unknown observation
when no windows are available.
Unscoped Codex windows of other durations remain in session JSON but have no bar widget.
For example, to leave out Fable
and other scoped Claude chips:

```toml
[bar]
widgets = ["codex_quota_5h", "codex_quota_7d", "claude_quota_5h", "claude_quota_7d", "cpu", "ram", "disk", "clock", "version"]
```

Selection affects the bar, its hover and accessibility, and the logo's room
reservation only. `eon session --json` still reports every window read for
each enabled provider, including hidden scoped windows.
Only reported windows appear; an idle Claude window can lack a reset time.
Hover and accessible labels report that absence. Narrow windows hide quota
chips one by one, the one closest to the tabs first, before tabs and controls.

Eon reads a provider's quota only while `[bar] widgets` lists at least one of
its quota widgets, checked
when a window opens, reopens or accepts a reload. A chip listed again shows its provider's
stored reading at once, if there is one, and otherwise appears after its
provider's next read.

Codex refreshes at least one minute apart and Claude at least five minutes
apart, also after a failed read or a chip shown again. Eon windows share one
Claude read every five minutes when the shared schedule is usable; otherwise
each keeps its own schedule. Older Eon versions still read on their own.
After the provider refuses a Claude read (for example a rate limit), the next
one waits five minutes plus up to one minute of jitter when it is the first
refusal since a successful read, then 30 minutes after a second refusal, an
hour after a third and two hours after each later one, until a read succeeds.
Only a success resets the count. A supervisor started from an older Eon still
writes its own, longer delay into the shared schedule, and the others honor it
until that supervisor restarts. A failed read keeps the last good
values; they become `old` once they are more than an hour old and disappear
after 24 hours. Each supplied reset expires its own window. Missing facts do
not mean zero usage. Sign-out is not detected; the last reading ages like any
other and disappears after 24 hours.

Each provider's last good reading is also stored in `$XDG_STATE_HOME/eon/quota`
(`~/.local/state/eon/quota` by default), shared by all your Eon windows, so a
newly started window shows it until its own read succeeds. The stored reading
holds only windows, remaining percentages, resets and when they were read.

Claude needs no status-line setup.

Existing Eon windows retain their running build after installation updates;
start a fresh window with the updated command to use this backend.

## Popups

Popup entries use the same file. Popup margins are fixed at 8 logical pixels
at the sides and 4 above and below; a `[popup]` table is ignored with a warning.

```toml
[popups.monitor]
command = ["btop"]
keybinding = "Alt+Shift+U"
label = "Monitor"
keep_alive = true
```

Built-in popup commands, labels and shortcuts are listed in the defaults above.
Every built-in popup accepts `keybinding` and `label`. Git, Agent, Commands and
Sessions also accept `command`, `enabled` and `keep_alive`, and
Agent's `command` may also be `"auto"`. Anima and Scrollback also accept
`enabled`, and they reject `command` and `keep_alive = true`. Editor also
accepts `enabled`. It uses `[editor] command` and keeps running when hidden.
Eon warns and ignores `command` and `keep_alive` in `[popups.editor]`.
Editor counts toward the limit of 32 enabled popup entries. If your
configuration already uses all 32, disable Editor with
`[popups.editor] enabled = false` or disable another popup.
Project has no
command of its own:
it shows the tab's Files on quick search, and Alt+Z hides it again when quick
search is already shown. It rejects `command`, `enabled = false` and
`keep_alive = true`, since it is required. A
custom popup needs `command` and `keybinding`; its `label` defaults to its ID
and its `keep_alive` to `true`.
On Linux with an agent configuration source, Agent's `auto` command uses the
supervisor build's `eon agent launch --popup` in the popup's own terminal and
tab launch directory. Source precedence is the same as [agent commands](#agent-commands).
An invalid or unreadable source, or a managed refusal, never retries through
the built-ins. Without a source, and on macOS, `auto` runs the first of
`codex resume --no-daemon`, `grok`, `opencode`, `pi` and `claude --resume`
available on the terminal PATH. Showing a live Agent reuses its terminal
without another choice. Built-in Codex popup commands disable its shared
daemon so its `eon` commands act for the Agent popup's own terminal (see
[Codex's shared daemon](USAGE.md#sending-to-a-terminal)).
Files opens Yazi with Eon's own popup keys on Alt+Shift+Y and reads no Yazi
configuration of yours. It is the tab's directory pair, so it is required: Eon
ignores `command`, `enabled = false` and `keep_alive = false` there, with a
configuration warning. For a Yazi that reads the `yazi` directory under Eon's
configuration root, add a custom popup such as `[popups.yazi]` with
`command = ["eon-yazi"]` and a free `keybinding`.
Anima opens a random animation and stops when dismissed or replaced.
Scrollback opens what the tab shows, its shown popup or else its selected
pane, in `[editor] command` (see [Editor](#editor)): the terminal's retained
history followed by its screen, as plain text in a private snapshot file that
Eon removes when the popup ends. Its key again dismisses it, which stops the
editor. The snapshot is limited to 8 MiB, the oldest lines dropped first, and
its first line says what was left out; a full-screen app's terminal shows only
its screen, and the first line says so. The bound, the plain-text layout, the
file's private 0600 mode in a 0700 directory, and its removal at the popup's
end are fixed for safety and by Orbit's history contract (ORB-C17).
`eon pane history TARGET --edit` opens Scrollback on TARGET from the CLI (see
[Reading a terminal](USAGE.md#reading-a-terminal)); with Scrollback disabled
it exits 3 with `unknown-popup`.
Commands searches and runs Eon commands, including **Check for updates** and
**Install update** (search `update`). These run `eon update --check` and
`eon update --install`, retaining results until Enter and using `[updates]`
target settings and the CLI's own target/source confirmation. With Commands
enabled, the Window menu offers **Updates** to open this popup. Its description
uses Commands' configured `label`. These fixed action names identify the
documented CLI operations; they add no shortcut or automatic check.
Sessions lists this window's sessions most recently shown first, the shown
one last, marking one that needs input: Enter shows the highlighted one, and
Enter on a name that matches none creates a session with that name.
Set `enabled = false` on Git, Agent, Anima, Scrollback, Commands, Sessions,
or a custom popup to disable it.

A `keybinding` you configure, on a custom popup or a built-in one, takes its
key first. A built-in popup's default key applies only while no configured
popup holds it; otherwise the built-in popup stays without a key, and Alt+/
lists it under its default key as overridden by the popup that holds it.
`keybinding = ""` leaves any popup, Project included, without a key.
`eon popup show ENTRY [--tab TAB]` starts an enabled entry without requiring
a keybinding, using the same configured command and lifetime. Existing
instances use `eon focus uN`; Project and Files share one instance. The
window still opens a new tab on quick search, and the Eon Bar's session
label opens Sessions.
Two configured popups on one key fail at load, except on a key reserved for
moving focus, where both load without a key. A popup shortcut on the key of a
built-in Eon shortcut takes that key, and Alt+/ lists the built-in one as
overridden.
Alt+/ and Alt+0 through Alt+9 are reserved and cannot be bound. Alt+H, Alt+J,
Alt+K and Alt+L are reserved for moving focus: a popup configured on one of
them still loads but gets no key, and Eon logs a configuration warning naming
the popup and the key.

Configuration uses physical names such as `Alt+Slash` on every platform.
Window labels use `/` for Slash and, on macOS, Option, Control and Command
for Alt, Ctrl and Super. These labels refer to the same physical chords.

To resolve clashes with terminal programs and agents that use `Alt+` chords,
remap or unbind popup entries here, use [input lock](USAGE.md#input-lock),
list fixed chords in `[keys] release`,
set `[session] number_keys = false` to release `Alt+Shift+0` through
`Alt+Shift+9`, or set `[sidebar] enabled = false` to release `Alt+Shift+H`.
See
[Keybinding clashes](USAGE.md#keybinding-clashes) for common clashes and
app-side workarounds.

Set `EON_CONFIG_HOME` to select another configuration root and `EON_RUNTIME_DIR`
to select another runtime root. Relative `EON_CONFIG_HOME` values resolve once
against the launch directory. Eon otherwise uses the standard XDG locations and
private per-user fallbacks.

## Editor

`eon edit -- FILE`, and Enter on a text file in Files, open the file using
`[editor] command`. The default `["hx"]` is Eon's packaged Helix. With
`open_in = "popup"` (the default), it opens in the tab's Editor popup: an existing
editor receives the file as a buffer and retains its other buffers and unsaved
edits; otherwise Editor starts with the file. Editor shows in place of Files.
`FILE:LINE[:COL]` works as in Helix, with an existing literal filename taking
priority. Set `open_in = "pane"` to open a new pane instead.

Directories, other editors, a disabled Editor popup, and a live Editor started
without Eon's listener use the new-pane path. That preserves the existing
editor's buffers. A build without packaged Helix (`no-helix`) has no default
command (see below); selected Neovim uses its Editor popup there too, while a
user's own `hx` uses a pane.
Any terminal editor works:

```toml
[editor]
command = ["nvim"]
```

`command` is a direct argv array, like a popup's: no shell runs it, so write
`["emacs", "-nw"]` rather than `"emacs -nw"`. A bare name resolves through the
terminal PATH, as popup commands do, and a relative path such as `./bin/ed`
against the tab's directory. Each edit reads the setting again, so a
change to `open_in` applies to the next file you open. Command and kind
changes govern fresh Editor starts; a live owned Editor retains its recorded
listener kind. When a fresh editor is not on the terminal PATH, the edit fails with `edit-command` and
opens no pane; Eon does not fall back to Helix.

In the package without Helix, while `command` is unset each edit runs
`$VISUAL`, or else `$EDITOR`, as they were set when the window started. Eon
splits the value on ASCII whitespace and runs no shell, so `nvim -p` works, and
an editor whose path has a space, or that needs quoting, goes in `command`.
The first nonblank value decides; a blank value falls through to the next. If
the editor it names is not on the terminal PATH, the edit fails rather than
trying the other. With neither naming an editor, the edit fails with
`edit-command` and names this setting; Eon picks no editor for you.
`eon config defaults` shows `command` as unset there.

`[editor] kind` accepts `"auto"` (default) or `"neovim"`. Auto recognizes
Eon's packaged Helix and a command whose original argv[0] basename is exactly
`nvim`, before PATH or symlink resolution. This also works without packaged
Helix, including an unset command resolved from `EDITOR=nvim`. Auto does not
inspect scripts or inner arguments such as `["env", "NAME=value", "nvim"]`.
Other wrappers opt in explicitly:

```toml
[editor]
command = ["my-nvim"]
kind = "neovim"
```

The wrapper must stay in the foreground and forward Eon's appended Neovim
arguments unchanged. With `open_in = "popup"` (default), Files and `eon edit`
start that tab's Editor or reuse its owned listener, including in the package
without Helix. Reopening a file selects its existing buffer without reloading;
different files open as separate buffers. Unsaved work stays in memory even
with Neovim's `hidden` disabled. A modified current buffer whose `bufhidden`
would unload, delete or wipe it refuses the switch; an unmodified dashboard
may follow its native wipe lifecycle. `open_in = "pane"` retains the configured
argv and opens a pane instead.

Neovim popup paths require UTF-8. An existing literal filename wins over a
final `:LINE[:COL]` suffix; otherwise numeric suffixes select a one-based line
and byte column (Helix uses its own native position semantics). Zero becomes
one and positions beyond the buffer clamp to its bounds. Invalid suffixes or
numbers beyond Neovim's signed 32-bit native position range remain literal
filenames. Initial opens and reuse interpret positions identically; an initial
position uses one generated `+call cursor(L,C)` within Neovim's ten-command
limit. A directory, including one found after interpreting a Neovim position,
uses the pane route with the original absolute file argument.

The Editor popup (Alt+Shift+O) uses the same editor resolution when it starts.
The editor keeps its buffers and working directory when the popup hides or
the tab directory changes. When the popup starts the packaged Helix, Eon
supplies its private listener socket. Remove Helix's own `--listen` option
from `[editor] command` to use the popup. The `open_in = "pane"` route can
still pass that option to Helix.

For Neovim, Eon appends its own `--listen` socket. `--listen`, `--server`,
`--remote*`, standalone `--`, `--embed`, `--headless`, `-l`, and
`-e`/`-E`/`-es`/`-Es` conflict with this route, as does an incompatible startup
command prefix. A prefix that supplies files, stdin (`-`), a tag (`-t`) or a
quickfix target (`-q`) is incompatible: it could select that target instead
of the requested file. Option values such as the filename after `-u` remain
valid. Under `auto`, a key-only popup runs that command without
Eon's listener; explicit `kind = "neovim"` refuses with `popup-command`
before another popup is displaced. For file opens with an enabled Editor and
`open_in = "popup"`, explicit `kind = "neovim"` also refuses these conflicts
with `edit-command` for fresh Editor starts from Files and `eon edit`.
Under `auto`, those edits use panes; `open_in = "pane"` passes configured
argv unchanged.
Neovim popup launches require a canonical runtime path without `:` because
Neovim interprets it as a TCP address; choose an `EON_RUNTIME_DIR` whose
resolved path contains no colon. Command and kind changes affect fresh
Editor starts; an existing listener keeps its recorded kind after recovery.
The private socket directory is mode 0700 and its versioned marker mode 0600.
Neovim's socket inherits its umask; Eon does not change its supervisor's umask.
Same-user processes can access the listener. Unsupported listener metadata,
including an old empty marker, refuses routing: save your work and restart
that Editor popup. Eon never restarts it automatically.

A live editor that is not listening fails with `editor-unavailable`; a refusal
fails with `editor-rejected`; a reply deadline fails with `editor-unresponsive`.
Each exits 1 and leaves workspace topology and selection unchanged. Connect is
bounded to 100 ms and the answer to 1 second. A lost reply or client timeout can
leave the file open already or later; Eon never retries that uncertain request
or starts a second editor. Only Helix's explicit native-listener timeout reply
confirms that queued work was abandoned; Neovim provides no cancellation
acknowledgement. A Neovim answer timeout explains that Editor may be waiting
for input and names its effective configured key, or names `[popups.editor]
keybinding` when no shortcut is active. Reveal Editor manually and answer any
prompt without resending the open. Errors do not roll back editor plugins or
autocommands. RPC receives at most 64 KiB in total, nesting depth 32 and 16
notifications: fixed trust/resource limits within the same deadline.
Neovim popup paths require UTF-8 before startup or sending. Helix's live
listener requires UTF-8, while its fresh argv and all pane routes retain Unix
filename bytes unchanged.

The Scrollback popup (Alt+Shift+E, or `eon pane history TARGET --edit`) opens
its snapshot file with the same command, in a popup instead of a pane. When
the editor is missing, it fails with `popup-command` and opens nothing.

- The editor must stay in the foreground until you are done. A GUI editor
  needs its wait flag, for example `["code", "--wait"]`; otherwise it returns
  at once, the popup ends, and Eon removes the snapshot.
- The snapshot is discarded when the popup ends. Dismissing or replacing the
  popup stops the editor and loses unsaved edits, so to keep edits, save them
  under another name first.
- Eon removes the snapshot's directory, but not state your editor keeps
  elsewhere, such as Neovim's swap, undo and shada files under
  `~/.local/state/nvim`.
- If a window's supervisor dies with a Scrollback popup open, the recovering
  supervisor adopts a still-running editor's snapshot and removes it when
  that terminal ends; the next start of that window's generation removes only
  snapshots no live editor holds. One whose generation never starts again,
  for example after an Eon update, stays until you log out on Linux, where
  the runtime directory is in memory. The macOS fallback under `/tmp` is not
  in memory, so there it stays until the system clears `/tmp`.
- Opening Scrollback reads the history and then starts the editor, each
  within its own deadline. On a machine too loaded to finish both within
  the client's 76-second exchange allowance, the window may report Eon as
  unavailable while the popup still opens.

Terminal startup has a fixed 15-second lifecycle budget for spawning Orbit,
publishing Ready and acquiring its management lease. A failed start gets a
separate five seconds to roll back. Recovery and Stop retain five-second
operation budgets. These bounds are not settings: they bound workspace
blocking and keep the CLI and window's waits consistent. The window reports
slow replies after two seconds while keeping its current workspace visible;
its exchange allowance is 76 seconds and Stop's is 81 seconds. A failed start
reports observed startup stages and whether cleanup completed. The command may
already have run, so repeat it only if replaying it is safe; never retry while
cleanup is unconfirmed.

## Managed tools

A terminal without an explicit command uses `[shell].command`, listed in the
defaults above. `eon-nu` selects pinned Nushell. Select managed
Bash with `eon-bash` in `[shell].command`. Any other shell, such as a host
`zsh` or `fish`, runs as an unmanaged command without Eon's integrations.
Native user shell configuration loads before Eon's guarded Starship, Zoxide,
Atuin, and Carapace integrations. Last, if your PATH has
[mise](https://mise.jdx.dev), managed Nushell and Bash activate it the way
`mise activate` does in your own shell, so a project's mise tools and
environment apply from the first prompt. Eon skips it when mise is missing,
fails, or your configuration already activates it, and `mise = false` turns
it off. Eon never installs mise or trusts its configuration for you.

When Eon activates Starship in `eon-nu` or `eon-bash`, its prompt omits the
leading blank line unless you have your own Starship config. Each prompt call
uses a packaged config containing only `add_newline = false` when
`STARSHIP_CONFIG` is unset and `$HOME/.config/starship.toml` is missing.
Starship does not use `XDG_CONFIG_HOME` for this file. Create
`~/.config/starship.toml` or set `STARSHIP_CONFIG` in your shell configuration
to use your own settings. In Bash, use
`export STARSHIP_CONFIG=/path/to/starship.toml`. Setting `add_newline = true`
restores the blank line. A config created while the shell is running takes
effect at the next prompt. Eon never edits your file or exports the fallback to other commands,
so Starship's configuration commands keep their native behavior. Other shells
and user-owned prompts receive no Eon Starship default.

Managed Bash reports command start and exit status through OSC 133 C/D marks,
including commands that fail or are interrupted. Pipelines report the shell's
final status. Eon preserves your prompt and tool hooks, and skips its Bash
marks when the prompt or its directly named hooks already contain C/D
reporting. Bash sends its marks to the prompt terminal or `/dev/tty`, so they
stay out of redirected output; with no terminal it emits none. Bash supplies
no prompt-start A mark. Managed Nushell uses its own
OSC 133 reporting, including when Eon starts from a VS Code terminal;
`TERM_PROGRAM` stays available to programs with its inherited value. Native
Nushell user autoload runs afterward and can override shell integration settings.
An OSC 133 opt-out in Nushell's `config.nu`,
`$env.config.shell_integration.osc133 = false`, is kept.
Eon disables Nushell's native OSC 133 before every prompt, including the first, when
stdout is not a terminal, so those marks stay out of pipes and captures.

Managed Nushell supports `clip copy` and `clip paste` through the native
clipboard. Interactive `eon-nu` terminals also provide `clc`
and `clp` aliases. User autoload files run afterward and can override them.
If no desktop clipboard is available, Nushell reports the error.

The profile also exposes `eon-yazi`, `eon-ya`, `eon-lazygit`, and `eon-lg`.
Inside terminals their unprefixed names resolve to the pinned tools.
Eon does not rewrite the parent PATH, aliases, shell startup files, or native
tool data paths.

### Helix

Packages with Helix also expose `eon-hx` and provide `hx` in their terminals.
In the `no-helix` package, `hx` resolves through your PATH.
Eon's Helix reads your settings from `$(eon config dir)/helix/config.toml`,
normally `~/.config/eon/helix/config.toml`. It does not read the host's
`~/.config/helix/config.toml`.

The nova-helix fork embeds these defaults, including when no user file exists.
They also apply to `eon-hx` outside Eon and standalone builds of the same fork.
Each row shows how to override that setting with its upstream value in your
Helix config:

| Setting | Fork default | Override to restore upstream |
| --- | --- | --- |
| `theme` | `"carbonfox"` | `"default"` |
| `editor.soft-wrap.enable` | `true` | `false` |
| `editor.bufferline` | `"multiple"` | `"never"` |
| `editor.cursorline` | `true` | `false` |
| `editor.color-modes` | `true` | `false` |
| `editor.cursor-shape.insert` | `"bar"` | `"block"` |
| `editor.indent-guides.render` | `true` | `false` |

For example, `theme = "default"` restores the upstream theme, and
`editor.soft-wrap.enable = false` turns wrapping off. Omitted settings keep the
fork defaults. Your global file (or `hx -c FILE`) overrides the embedded file;
trusted workspace `.helix/config.toml` settings override it in turn, including
when no user file exists. Workspace trust policy stays global. Session changes
through `:set` or `:toggle` apply last. Helix leaves your config files intact;
`:config-open` opens your file and `:config-reload` reloads the layers.

Carbonfox needs true color; Helix falls back to its default theme without it.
`editor.true-color = true` overrides detection when your terminal supports it.
Auto-save is disabled unless your configuration enables it. Eon's Helix is built
without Steel, so it never runs `helix.scm` or `init.scm`, whether they are in
your config directory or in a repository's `.helix` directory.

#### Reveal the current file in Files

To bind Alt+R in normal mode, add these settings to your Eon Helix
`config.toml`. If `[editor]` or `[keys.normal]` already exists, add the key
under its existing table. Run `:config-reload` in Helix to load the settings.

```toml
[editor]
shell = ["/bin/sh", "-c", '''
case "$1" in
  "eon-files-reveal: "*)
    file=${1#eon-files-reveal: }
    if [ -d "$file" ]; then
      printf '%s\n' 'Reveal in Files needs a file buffer.' >&2
      exit 1
    fi
    exec eon files reveal -- "$file" >/dev/null
    ;;
  *) exec /bin/sh -c "$1" ;;
esac
''', "helix"]

[keys.normal]
A-r = ':sh eon-files-reveal: %{file_path_absolute}'
```

The recipe passes the absolute filename as one argument to `eon files reveal`.
If you use a different shell for other Helix commands, keep it in the fallback
line. Choose another key in `[keys.normal]` if you bind Alt+R in Eon.

## Installation updates

`updates.nix_profile` and `updates.nix_entry` are unset by default. Set both to
choose the absolute Nix user-profile path and its exact entry name for
`eon update --check` and `--install`. A CLI `--profile ABS --entry NAME`
replaces this pair. With neither, Eon discovers the invoking installation;
inside an Eon terminal, package update modes first run the external `eon`
selected by absolute PATH entries, skipping Eon's internal command directories.
That CLI validates its own installation, even when it selects a newer build
than the window. Without an external launcher, configure or name the target.
An incomplete pair, a relative profile, a `nix_profile` path containing `..`,
or an empty name fails without mutation.
`updates.bundle_prefix`, also unset, selects an existing absolute installer-owned
Linux prefix instead of the Nix pair. A CLI `--prefix ABS` or complete profile
pair replaces the entire configured target. `updates.allowed_signers` is unset
until independent signer setup supplies an absolute OpenSSH allowed-signers
file outside the managed prefix; direct Install refuses without one. Eon never
supplies or fetches a key.
Relative paths, parent traversal and mixed targets fail. A CLI Nix pair bypasses
configuration loading. Other calls, including `--check --prefix`, read
configuration and the signer setting; parse errors fail with
`invalid-configuration`. A CLI prefix bypasses
configured target validation while retaining the signer setting.

```toml
[updates]
nix_profile = "/home/me/.local/state/nix/profiles/profile"
nix_entry = "eon"
```

For a managed prefix:

```toml
[updates]
bundle_prefix = "/home/me/.local/share/eon/install"
allowed_signers = "/home/me/.config/eon/allowed_signers"
```

See [independent signer setup](INSTALLER.md#install-a-release).
Checks run only when requested. Install requires consent; in JSON or without
a terminal, pass `--yes` and set an explicit or configured target. Channel,
automatic installation/restart and custom manager commands are fixed by the
installation's ownership and live-work safety rules. Cancellation gives only
the helper's children a fixed five-second termination grace before killing
and reaping them. This safety budget bounds shutdown and lets Nix release
resources; it is not a configurable build timeout. The shared Nix daemon and
running Eon processes receive no signal.

The test-gated in-place Update Commit gives helper children, quota workers,
window scans, pane observers, attention and the notification worker up to
1,250 ms to settle within a 1,500 ms monotonic quiesce budget. In the same
budget, and at the same time, management requests already sent to Orbit get
up to 1,000 ms to finish. Commit sends no new request and no Stop, and it
reads each reply only as the request that asked for it. Commit refuses when a
reply is late, malformed or not the request's answer, or when a terminal sent
something no request asked for. The current Runtime then finishes that
request as usual. Target validation has its own deadline. A live detach
notice or a pending detach action requires settlement in the current Runtime
before Commit can proceed. Refusal leaves the current Runtime serving the
window. Only standard I/O and the explicitly carried resources may survive
exec. Runtime checks other open descriptors for close-on-exec on Linux and
macOS and refuses if it finds an unexpected inheritable descriptor.

Every ordinary accepted `eon.sock` client, including one accepted while a
terminal starts, has 250 ms from its accept to send its whole request and
2,000 ms to take its whole reply. A request still incomplete at 250 ms gets a
`malformed-action` failure and is not run, however slowly its bytes kept
arriving. The action's own work has its own limits. These budgets apply
before an Update Commit's quiesce budget starts, and Commit admits no new
client: clients still waiting in the listener's backlog stay there for the
Runtime that serves the window next. These fixed bounds keep one slow client
from holding the window's other clients and its Update.

After in-place Update import, quota reads using a local schedule wait at
least one ordinary interval from import, 60 seconds for Codex and 300
seconds for Claude or Google. Usable shared Claude and Google schedules
continue to govern reads. The local delay lives in memory and leaves the
shared schedule untouched. Stored readings remain available during the
delay. These fixed bounds preserve update safety and provider cadence.
