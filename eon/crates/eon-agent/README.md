# rota

Picks which coding-agent subscription runs your next agent session.

You may pay for several coding-agent subscriptions: Claude Code, Codex,
Antigravity, OpenCode and others. rota looks at each one's remaining quota and
how many sessions it already runs, then starts the next agent session on a
backend that has room, with the model and effort level the task asks for.

## Usage

Build with `cargo build --release` (Linux, Rust 1.96 or later). Copy
`config.example.toml` to `~/.config/rota/config.toml` and replace its placeholder
paths, commands, models and example percentages.

```text
rota launch [--role implement|review|lead] [--level medium|high] [--only BACKEND] [--project NAME] [--name NAME] [--add-dir DIR]... [--wait DURATION] [--dry-run|--json] <prompt-file>
rota launch --popup [--only BACKEND] [--dry-run]
rota run --brief FILE --name NAME --timeout DURATION [--role implement|review] [--only BACKEND] [--project NAME] [--cwd DIR] [--effort LEVEL] [--add-dir DIR]... [--wait DURATION] [--json]
rota ps [--json] [--all]
rota slot run [--class NAME] [--quiet-exempt] [--front] -- COMMAND...
rota slot ls [--json]
rota retire NAME [--project P] [--bead ID] [--target-only] [--dry-run] [--json]
rota status [--json]
```

Launch defaults to `implement` and `high`, runs from the current folder and
replaces itself with the selected CLI. It tries backends in configured order and
takes the first one whose guard, account check, quota limits and running processes
admit it; nothing rotates. It reads candidates one at a time and stops at the
first available one, so later backends show `not checked`.
`launch --popup` shares that admission but takes no prompt file. It uses an
optional backend `popup_command` argv (whole `{cwd}` substitution, no `{prompt}`)
or the configured high executable with its built-in popup arguments: Codex
`resume --no-daemon`, Claude `--resume`, Grok/OpenCode/Pi no extra arguments.
Other executable basenames need `popup_command` to be eligible in popup mode.
Ordinary launch still uses its level command and requires a prompt file.
For a real popup-mode failure with terminal stdin and stderr, the diagnostic
remains until Enter or EOF, then returns its original status. Dry-run,
nonterminal calls and ordinary launch return immediately.
Only implementers without `--only` can use the configured fallback, reported as
`FALLBACK`. Exit codes are 75 for a hold, 69 for a refusal, 2 for invalid
input/config, and 1 for I/O or launch failure. Status reports availability
for both roles. A role a backend runs only through `rota run` shows as
`available for rota run`.

`--dry-run` prints the pick and shell command, showing the prompt as `$(cat file)`.
It does not write the state, log, lock or trusted settings, run preparation hooks,
or refresh an account. Quota and guard commands must themselves be read-only.
Status also skips refresh and writes no rota files.

A backend that sets `limits` admits one launch per quota observation. Every
reading reports its account and `observed_at`, when the provider data was
fetched. An admission records that time for the account in `admissions.json`
beside `state_file`; a later admission on the same account needs a later
`observed_at`, and a reading older than `[backends.quota] max_age_seconds`
(default 600) holds even when unspent. On a repeat or a reading too old, a real
launch runs the reader's `refresh` (when configured and below the cap), reads
once more, and holds with
`reading observed at T already admitted a launch on account ACCOUNT; hold for a fresh reading`
or `quota reading on account ACCOUNT is N s old (max_age_seconds 600)`
when no later time arrives. `launch` and `run` share the record, so an
observation spent by one holds the other. A reading without `observed_at`
(`quota reading on account ACCOUNT has no observed_at`), one observed in the
future, a provider failure reported beside retained facts
(`quota unreadable on account ACCOUNT: provider failure (REASON)`), and an
unreadable ledger all hold a limited backend, with two exceptions beside
`eon_claude` facts. A failure older than the reading is superseded by it. A
newer failure leaves them stale: they still admit, under the backend's limits and once
per observation, while under 30 minutes old when admission reads them,
whatever `max_age_seconds` says. Output marks them
`stale 18m (last read failed: REASON)`, and JSON readings carry
`source.stale` (`{"age_ms", "failure"}`). At 30 minutes or older a real launch
reads the facts once more, without `refresh`, and holds with
`...; the last good reading is N s old (a stale reading admits under 1800 s)`. Dry-run and status show these
holds but never write the ledger or refresh. If `admissions.json` is ever
corrupt, delete it: each account's last observation may then admit once more.
Backends without limits consume no observations. CodexBar readers still check
the observation age on those backends.

`--role lead` marks a lead launch, by convention rather than as a security
boundary. Backends opt in with `roles = ["lead"]` and are tried in configured
order, with no fallback. A lead launch reads no quota (the reader never runs),
consumes no observation and checks no limits; enabled state, guard, account
check and cap still apply, and the decision log records `"role": "lead"`.
`rota run` refuses `--role lead` (exit 2). Status shows a `lead` column computed
from those access checks alone.

`--wait DURATION` (`300`, `300s`, `30m` or `2h`; at most 24h) retries a held
admission until it admits or the deadline passes. Each attempt re-reads the
configuration, takes the admission lock only while it works, and reads fresh
quota; between attempts it sleeps up to `wait_retry_seconds` (default 60),
never past the deadline. Only the first attempt may run a backend's `refresh`.
Waiters are not ordered, and one observation still admits one launch. A waiter
prints `held: REASON; retrying for up to DURATION` once and repeats its reading
lines only when they change; at the deadline it exits 75 with
`REASON; still held after waiting DURATION`. Without `--wait` an admission
makes one attempt. The popup takes no `--wait`.

`launch` and `run` exit 75 for a hold that waiting can change: quota, a spent
observation, a cap, a guard, account check or required file, a project cap, a
busy admission lock, or the `--wait` deadline. They exit 69 for a refusal
waiting cannot change, decided before any reading: an unknown `--project` or
`--only` backend, no backend with the role, or every candidate disabled,
excluded for the folder or unable to take the options. Invalid input or
configuration exits 2 and operational failure 1.

`rota run` runs one headless builder or reviewer and waits for it.
It checks guards, account checks, quota limits and caps like
`launch`, with no fallback, then runs the backend's `run` command for the role
in `--cwd` (default: the current folder). It does not apply `trusted_settings`
or `--level`. Its stdin is the brief file or
`/dev/null`, never rota's own. Output goes to a log under a run directory named
by the session, `NAME-N` with the first free `N`. `--timeout` takes seconds or
`Ns`, `Nm`, `Nh`, at most 30 days, and counts from the moment the run starts.

Stdout carries newline-delimited JSON. Both receipts have `schema: 1`,
`event`, `id` (the session ID), `name` (the requested run name), `backend`,
`pid` (the scope leader), and absolute `log` and `report` paths. A `started`
receipt follows registration. A `finished` receipt follows cleanup and the
final record write, with `outcome` and `exit_code` added. The report path
is where the provider can write a report. Check that file before using it.

`run --json` and ordinary `launch --json` also write one stdout JSON error
for a final admission hold or refusal, with `error.code` (`agent-unavailable`
for 75, `agent-unsupported` for 69), `error.detail`, and `error.reasons`
(a list of backend and detail pairs, empty if none were collected).
Stderr retains the diagnostics. A successful launch execs the agent with the
caller's stdout. Launch's JSON option is unavailable with `--popup` or
`--dry-run`.

`rota run` needs Linux 5.14 or newer with cgroup v2 and a systemd user manager,
systemd 254 or newer (tested on 255; systemd-run gets `--expand-environment=no`,
so arguments stay literal on 258 and later too). It starts the command with
`systemd-run --user --scope` in a transient unit `rota-ID-HEX.scope`, where HEX
is 128 random bits, so the unit's cgroup holds every process the run starts,
whatever their process group, session, parent or threads. Before it claims a
session, rota starts a throwaway scope that checks its own `cgroup.events` and
`cgroup.kill`; if that fails or takes more than 10 seconds, rota exits 1. A run
always gets rota's own `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS`, the
variables that reach the manager, and `[run]` may not unset or set them.

rota acts on the unit only once it has seen the run's main process inside it,
so a name that was already taken is never signalled. When the run's main
process exits, at the timeout, or when rota gets SIGINT, SIGTERM or SIGHUP, rota
sends SIGTERM and SIGCONT to every process in the unit's cgroup, waits up to 5
seconds for it to empty, then sends SIGKILL (and writes `cgroup.kill`) for up to
10 seconds. rota signals the processes directly through pidfds, never through
the systemd manager, so a stalled manager cannot delay cleanup, and it skips any
listed PID that has since left the unit. The run ends only when
`cgroup.events` confirms the cgroup is empty. If rota cannot confirm that by
then (processes stuck in uninterruptible I/O, or a cgroup or process membership
it cannot read) the run is abandoned: rota prints the PIDs it can see, records
them, and exits 125, never 0. If systemd-run fails before the command starts
(rota read the main process's cgroup and it is not the unit's), the outcome is
`failed` and rota exits 1; the log says why.
rota exits with the run's status: 128+N when a signal N ended it, 124 after a
timeout, 130 after an interruption and 125 when it abandoned processes. If the
final record write fails, rota exits 1 or retains 125 if cleanup is unknown.
Before anything starts, a hold exits
75, a refusal waiting cannot change 69 and an unusable option or brief 2, as for
`launch`. A provider can also exit 1, 2, 69 or 75. The `started` receipt and
the finished `outcome` distinguish a started run from an admission error.

Each run is recorded in the session registry: `sessions/ID.json` beside
`state_file`, naming the run's `unit`, rewritten when the run ends with `finished` (Unix seconds),
`outcome` (`exited`, `signaled`, `timeout`, `interrupted`, `failed` or `abandoned`) and
`exit_code`; an abandoned run also lists `abandoned_pids`.
Run directories are `runs/ID/` beside `state_file`, holding `log` and the
`report.md` path given to the command. Readers ignore fields they do not know.
Writers hold `sessions/.lock` and merge their fields into the record as it is
on disk, so fields another command adds during a run are kept. A record that
exists but cannot be read or parsed is never replaced. `rota run` takes the lock
before the run starts; the final write waits for it at most 10 seconds.

`rota ps` lists the registry's sessions that have not finished, oldest
first; `--all` adds finished ones. Each has a state: `running` while its
process is alive, `dead` when the process is gone but the record never got
`finished` (rota itself was stopped), or `finished`. A process is alive while
`/proc/PID` exists and, for a record that names a `unit`, the process is in
that unit, so a reused PID never passes for the session. The text form prints
one line per session: ID, state, backend, role, PID, age, folder, log, and the
pane, outcome and exit code when the record has them. `--json` prints the
records as stored, each with `state`, `alive` and `age_seconds` (seconds since
`started`) added. ps reads the registry without its lock and writes nothing.
A record it cannot read or parse is reported on stderr, the others are still
printed, and rota exits 1.

`rota slot run` waits for a machine slot and runs the command with its original
arguments and inherited stdin, stdout and stderr. Use the same state directory
and slot configuration from each checkout to share slots. The oldest eligible
waiter gets each free slot. By default Nix builds and profile commands use slot
2 when two or more slots are configured. Without an explicit `--class`, rota
skips the queue for `nix eval`, `nix path-info` and `nix build --dry-run`,
including a single Nix command inside `sh -c`, `bash -c` or `timeout`. A
nested invocation runs directly when its inherited slot and grant token match
a locked holder record, including across PID namespaces. The record stores
the SHA-256 digest of the token. A command returns
its exit status, or 128 plus the signal that ended it. If rota cannot register
a ticket, it exits 3 and prints
the queue directory that needs write access.

`rota slot ls` lists holders by slot and waiters by priority and arrival time.
The JSON form returns `holders` and `waiters` arrays. Each waiter has a
`slots` array with its current eligibility. An empty array means quiet policy
is holding the waiter. The queue, holder records and `log.jsonl` live under
`slots/` beside `state_file`. The log records queue, grant and exit times.
`slot ls` reads `/proc/locks` and matches locks to files by device and inode.
This works on ext4. Filesystems that report different device numbers through
the two interfaces can omit locks from the listing. Listing works with
read-only state and exits 1 if the lock table cannot be read. Run it from the
host PID namespace to see locks across sandboxed agents. Slot locks stay with
the rota wrapper and close on exec. A descendant that
outlives the command cannot retain a slot lock.

## Configuration

`ROTA_CONFIG` selects a config file; otherwise rota uses
`$XDG_CONFIG_HOME/rota/config.toml`, defaulting to `~/.config/rota/config.toml`.
Paths in TOML are literal: use absolute paths; shell variables and `~` are not
expanded. Keep personal state and logs outside the repository.

The example keeps backend permission handling: agy uses `--sandbox`, and
interactive Codex uses `-s workspace-write -a on-request`. Codex TUI commands
also use `--no-daemon` so command execution retains the launching terminal's
environment; include it in custom TUI and resume commands too. The headless
`codex exec` examples retain workspace-write for implementation and read-only
for review; they do not use the TUI-only daemon flag. Headless stdin is not a
terminal: denied work must fail rather than be retried with permission bypass.
Other example backends retain their own permission settings. Eon does not
create or attest to a backend sandbox; backend configuration and the separate
`trusted_settings` workspace-trust grant still matter.

Workspace-write can deny access to Eon's host socket and writes to state outside
the workspace. Codex may request approval for an operation or refuse it; an
approval is not a guarantee of socket access. For the state directory, see
[sandboxed slot access](#sandboxed-slot-access). Do not grant broad host-socket
access to work around a denial: Eon's control and helper sockets can provide
execution outside the agent sandbox.

If you deliberately configure permission bypass, give that backend an explicit
name such as `code-bypass` or `agy-bypass` and select it with `--only NAME`.
For example, renaming a Codex backend to `code-bypass` and replacing its sandbox
and approval arguments with `--dangerously-bypass-approvals-and-sandbox` grants
command execution without sandboxing or confirmation prompts. Retain
`--no-daemon`; external isolation is your responsibility. agy's
`--dangerously-skip-permissions` instead auto-approves tool permission requests.
These are deliberate edits to your commands, not options enabled by the example.
Inspect the choice with `rota status --json` and
`rota launch --only code-bypass --dry-run task.md` before launching.
Human status prints the configured name, JSON status exposes it as `backend`,
and dry-run prints it as `pick`. Names describe your choice; Eon does not infer
or enforce permissions from them. A backend enabled for normal selection, fallback
or a popup pin can also be selected without `--only`; those are configuration choices.
Custom popup and headless argv remain your explicit command choices too.

- `backends` array order is admission order: the first available backend is
  picked every time, and nothing rotates. `roles` (`implement`, `review`,
  `lead`) controls eligibility; the fallback backend has an empty role list.
  Disabled backends never run checks.
- Optional top-level `wait_retry_seconds` (1 to 3600, default 60) is the
  longest sleep between `--wait` attempts, re-read on every attempt. Older
  builds reject the whole file when it is present: upgrade every binary that
  reads a shared config before setting it, and remove it before rolling back.
- Optional top-level `popup_backend = "main"` pins `launch --popup` to an
  implement backend or the configured fallback. Omission keeps dynamic
  selection; explicit `--only` overrides it with its existing role restrictions.
  The pinned backend keeps its checks and environment, and refusal never
  selects another backend. Claude resume reads `CLAUDE_CONFIG_DIR`, defaulting
  to `~/.claude` under its effective home. Pin that store's backend to resume
  there; its `unset_env` can clear an inherited alternate store. Histories are
  never merged. Ordinary launch/run selection is unchanged.
- `commands.medium` and `commands.high` are argv arrays, with one whole argument
  `{prompt}` and optional `{cwd}` arguments. They run directly, without a shell.
  OpenCode uses `opencode mini -m MODEL#VARIANT --prompt {prompt}`.
- `env` adds environment variables to commands and probes, including proxy base
  URLs. `unset_env` removes inherited variables, for example on the main fallback.
- `guard` must exit successfully. Configure it to verify the command guard is
  installed **and hooked into that backend**. `required_files`, `account_check`
  and `excluded_roots` support credential/account checks.
- Custom `quota.command` returns data for `quota.parser`: `agy` (tab-separated usage),
  `claude_text` (five-hour/seven-day used percentages, with `None` for missing,
  then an optional `observed_at=MS` token), `claude_json` (usage endpoint JSON),
  `codexbar` (CodexBar CLI usage JSON), or `remaining_json` (object mapping
  window names to remaining percentages).
  `claude_json` and `remaining_json` take an optional top-level
  `"observed_at": MS`; in `remaining_json` that key is reserved, never a window.
  `MS` is Unix milliseconds, an unsigned integer; anything else fails the read.
  `agy` output carries no time, so an `agy` backend with limits always holds.
  `agy` reads the rows of one model group: `agy_group` names it and defaults to
  `Claude and GPT models`. The group needs a weekly row; its five-hour row is
  optional. A limit that names `five_hour` still refuses a group without that
  row unless `absent_windows` declares it. A `disabled` value reads as 0% remaining,
  and each row's RFC 3339 reset time is kept when present.
- `parser = "codexbar"` reads one account through CodexBar. For Codex, set
  `command = ["codexbar", "usage", "--provider", "codex", "--source", "cli", "--format", "json", "--json-only"]`
  in `[backends.quota]`. For Claude, use `--provider claude --source oauth`.
  CodexBar handles credentials. Use `five_hour` and `seven_day` in limits
  and `absent_windows`, and `seven_day_<id>` for named Claude weekly extras.
  Quote non-ASCII keys, for example `"seven_day_é" = 50`. Claude's unnamed
  tertiary window has no model key. Configure the general and named scoped
  limits together when both apply. A `"*"` limit requires each base window
  unless declared absent. It checks usable model extras. Set an explicit
  model limit to hold when that model's extra is missing or unusable.
  An error, nonzero exit or stale reading holds. `status --json` reports
  available provider ID, email and label in
  `reading.source.identity` as `account_id`, `account_email` and `account_label`.
  `reading.source.account` is the configured account used by the ledger.
  Upgrade all binaries that read a shared config before opting in, since
  older binaries reject this parser. When switching a native reader, update
  duration keys such as `300m` and `10080m` in limits and `absent_windows`
  to the corresponding named keys. Restore the old config before rolling
  back a reader. For a Claude account Eon already polls, use `eon_claude`
  through `eon agent` to share its polling and back-off, including after
  a rate limit.
- `eon_codex` / `eon_claude` read the invoking Eon caller's Runtime facts in
  process, using the same projection as `eon session --json`. Their optional
  `command` is ignored by Eon and remains readable by rota in shared config.
  `timeout_seconds` (default 30) covers the whole native read, including
  discovery and inspection; the caller's window gets at most half of it. A
  provider that window does not supply (it runs another EONW, cannot be
  reached or answer in time, or has no facts for that provider) is read from
  the quota store Eon's windows share, without calling the provider: Claude's
  recorded failure comes with it and an unreadable failure record refuses it,
  and a stored Codex reading counts only while at most 120 s old. Native
  readers never invoke an incompatible generation's recorded launcher. Each
  native reading names its source as `reading.source.via` in `status --json`
  (`"window ID"` or `"store"`) and as `via SOURCE` at the end of the launch
  candidate line on stderr, also when that source's facts are stale or only a
  failure; a failure without usable facts reads
  `quota unreadable on account ACCOUNT: provider failure (REASON); PROBLEM`.
  A provider no source has fails with
  `quota unreadable on account ACCOUNT: window: REASON; store: REASON`. The
  host's facts may carry `quota_source` (each provider's source) and
  `quota_unreadable` (why no source has a provider); without them a reading
  names no source.
  They require the `eon agent` entry point; the standalone test driver has no
  native source. Backend credential environment overrides do not change the
  supervisor's quota facts. Their snapshot's `observed_at` is the reading's
  time. While Eon reports `codex_quota_failure` / `claude_quota_failure`, a
  backend with limits holds with
  `quota unreadable on account ACCOUNT: provider failure (REASON)`.
  Custom reader commands have a configurable timeout covering output collection,
  and a 1 MiB output limit. Optional `refresh` runs only during a real launch or
  run, below the backend's cap. It can run after a failed read or when the
  reading is spent or older than its `max_age_seconds`, then reads once more.
- `limits.implement` / `limits.review` compare `min_remaining` inclusively and
  `max_used` exclusively. `strictly_above_min` makes the minimum exclusive.
  A window a limit names must be in the reading, or the role holds
  (`missing quota window W`), unless `[backends.quota] absent_windows` declares
  it; a declared-absent window that does appear is checked normally. `"*"`
  checks every returned window. Eon window keys are `DURATIONm/INDEX` to preserve
  scoped windows with the same duration; a key `DURATIONm` without an index,
  in limits or `absent_windows`, matches every window of that duration, so
  `min_remaining."10080m" = 3` covers every weekly window in any order.
  `absent_windows` entries are nonempty, unique and not `"*"`. Lead launches
  check no limits, and `limits.lead` is a configuration error.
  `allow_missing` is an unknown key: a file that sets it fails to load, and
  the error points to `absent_windows`.
  Eon snapshots must report `state = "fresh"`;
  missing, stale, error and unknown snapshots refuse. Every quota hold names the
  account. A refused role lists every window under its limit with its value, the
  rule and the threshold the rule needs (`max_used < N%` admits strictly below,
  `min_remaining >= N%` is the inclusive floor, `min_remaining > N%` the strict
  one), adding the reset time when the reader reports one, for example
  `quota limit on account sclaude: five_hour used 55% (max_used < 50%, resets 2026-11-17T08:00:00Z)`
  or `quota limit on account uclaude: 10080m/1 left 2.9% (min_remaining >= 3%)`;
  a missing window reads `missing quota window seven_day on account sclaude`.
- `[backends.quota] max_age_seconds` (a positive integer, default 600 on a
  backend that sets limits or uses CodexBar) holds older readings as unreadable,
  even when the observation was never spent. Rota converts CodexBar's
  RFC 3339 `usage.updatedAt` to Unix milliseconds.
- `account = "NAME"` names the subscription account a backend draws on; it
  defaults to the backend's `name`. Backends sharing one account declare the same
  `account`, and so share its spent observations. A backend that sets limits
  must declare `account`.
- A backend without a quota reader requires explicit `allow_unknown_quota = true`.
  `allow_unknown_quota` is invalid on a backend that sets limits.
  An unsuccessful configured reader always refuses. The example disables OpenCode
  until a model and guard are chosen.
- `cap` scans Linux `/proc` for an executable basename and optional environment
  values; account-directory aliases are canonicalized. `exclude_args` omits usage
  probes. Counts include only the matching processes of the user running rota,
  regardless of role; a process that exits or changes owner mid-scan is
  skipped, and a live matching process whose environment or working directory
  cannot be read fails the check. A lock beside
  `state_file` serializes rota launches through exec; independently started CLIs
  are observed by the process scan rather than controlled by this lock. Caps are
  rechecked before refresh and exec, including resolved executable identities.
  Set `runtime_executables` to the actual binary paths behind wrappers that change
  argv[0]. For an interpreted CLI, set the executable to its interpreter basename
  and `script` to the absolute CLI script path to distinguish other interpreter jobs.
- `state_file` anchors the state directory: its lock is `state_file.lock`, and
  `admissions.json`, `runs/`, `sessions/` and `slots/` sit beside it. Nothing is
  written at `state_file` itself. `admissions.json` is
  `{"version": 1, "accounts": {"ACCOUNT": {"observed_at": MS}}}`, written
  atomically under the lock; a failed write refuses the launch (exit 1) before
  its command runs. `log_file` receives JSON lines with time,
  folder, role, level, decision and readings, without prompt contents. Fallback and
  refusal are explicit log decisions. `trusted_settings` atomically appends the
  folder to `trustedWorkspaces`, preserving other JSON keys and the file mode.
  `prepare` is an optional list of checked commands run just before exec.
- Level commands (`[backends.commands]`) take one whole `{prompt}` argument,
  `{cwd}` and `{name}` (from `launch --name`) inside any argument, `{add_dirs}`
  (repeating the backend's own optional `add_dir` template once per
  `launch --add-dir DIR`) and `{overlay}`. None may be the program. A backend
  whose command needs `--name`, or lacks `{add_dirs}` when `--add-dir` is
  given, is skipped with that reason, and the launch is refused (exit 69) when
  no backend remains. `popup_command` takes none of `{prompt}`, `{name}`,
  `{add_dirs}` or `{overlay}`.
- `[projects.P]` profiles (`max`, `[projects.P.backends.B] max`,
  `[projects.P.roles.R]` with `max`, `guard` and `overlay`) apply to launches
  and runs selected with `--project P` or made in P's `repo` (any folder or
  linked worktree). Caps count live agents: an admission records its process
  (PID and start time) in `agents/` beside `state_file`, and a record counts
  while that process, or a run's leader, still runs. A reached cap or a failing
  guard holds (exit 75) before any quota is read, naming the live PIDs or the
  guard's first stderr line. Overlays are keyed by the command's executable and
  apply to every backend the role reaches that runs it; a role with overlays
  picks only executables it names. Leads and the popup are never capped. The
  full rules are in Eon's
  [agent projects](https://github.com/Yazelix/eon/blob/edge/docs/CONFIGURATION.md#agent-projects) section.
- `[backends.run]` makes a backend usable by `rota run`. `commands.implement`
  and `commands.review` are argv arrays; a role with a command can run headless
  even when `roles` leaves it out, and its quota limits apply. There is no
  `commands.lead`. `{brief}`,
  `{cwd}`, `{report}`, `{effort}` and `{name}` (the `--name` value) are replaced
  inside any argument; `{add_dirs}` is a whole argument, never the first, that
  repeats `add_dir` (an argv fragment with one `{dir}` in total, such as
  `["--add-dir", "{dir}"]` or `["--add-dir={dir}"]`) once per `--add-dir`, and
  `{overlay}` takes the project role's overlay. `stdin` is `"brief"` or `"null"`;
  with `"null"` every command must name `{brief}`. `--effort` must be one of
  `efforts` and defaults to `default_effort`.
- `[run]` changes the environment of headless runs only, never of checks.
  `unset_env` removes inherited variables, where a trailing `*` matches a name
  prefix, then `[run.env]` sets variables, then the backend's `unset_env` and
  `env` apply. Use it to keep a run away from a desktop or terminal session.
- `[slots]` sets the slot `count` (default 2), the child nice increment
  (default 19), and child environment variables in `[slots.env]`.
  `[[slots.classes]]` lists classes in match order, each with `name`,
  `slots` and optional `match_argv`. A pattern matches consecutive arguments
  or words in the joined command, including a shell command argument. The
  final class omits `match_argv` and is the default. `--class` selects a class
  explicitly. `--front` jumps ordinary waiters and can use any active slot.
- During a cutover, `slots.lock_files` lists the existing absolute flock paths
  in slot order. Setting `slots.legacy_dir` requires this list. The extra
  lock path is optional and comes last; without it no extra slot is granted.
  Swap the old wrapper to a shim before starting native runs. The native
  scheduler honors outstanding legacy tickets, and both versions share the
  locks while the old wrappers drain. The shim translates the old overrides
  to `--front` and `--quiet-exempt`.
- Rota checks the optional `slots.legacy_dir` policies every five seconds.
  `extra-slot-until` enables slot `count + 1`. `one-slot-until` or a
  `slot2-stalled` marker narrows all classes to slot 1. `quiet-until` holds
  new grants except for `--quiet-exempt`, which also jumps ordinary waiters.
  The timed files contain Unix epoch seconds. A waiter under a directory in
  `yield-roots` yields on each slot to eligible higher-priority waiters for
  up to `yield-max-minutes` (default 30 minutes), then resumes FIFO by its
  original arrival time.

### Sandboxed slot access

With the example's Codex workspace-write sandbox, add
`--add-dir /PATH/TO/state` to permit writes to the directory containing rota's
`state_file`. Grant only the required state directory; this does not grant
access to Eon's host sockets.

The permission-profile recipe below requires a Codex version whose own help
advertises `--permission-profile`. Codex 0.169.0 rejects that option, including
when combined with `-s workspace-write`; use `--add-dir` with this example
instead. Do not assume the two permission interfaces can be combined.
For a supporting version, these settings extend an existing profile named
`rota` with a writable state root:

```toml
[permissions.rota.workspace_roots]
"/PATH/TO/state" = true

[permissions.rota.filesystem.":workspace_roots"]
"." = "write"
```

Select the profile with `codex --no-daemon --permission-profile rota` and verify it before
cutover by running `rota slot run -- true` in the sandbox. See
[Codex permission profiles](https://learn.chatgpt.com/docs/permissions#define-and-select-a-profile).

Eon's `eon agent slot --config FILE run -- COMMAND...` and `ls` use a dedicated
slot configuration without exporting a config override to the child.
`slots.local_runner` and `[slots.boat]` belong to Eon's slot route and act only
under `eon agent`. Standalone `rota` warns that they need `eon agent` and runs
every slot command on a local slot. Under `eon agent`, `slots.local_runner = []`
keeps native admission; a trusted nonempty command prefix owns local queueing. It receives original queue-age/class metadata and
must acquire local capacity before its supplied payload runs. Captured remote
source identity is checked before local admission and again before execution;
a changed/unverifiable source refuses with125. No automatic post-start retry.

Under `eon agent`, the optional `[slots.boat]` route runs permitted headless builds with no caller credentials on a
Boat VM using a separate lease; see Eon's [configuration reference](https://github.com/Yazelix/eon/blob/edge/docs/CONFIGURATION.md#remote-heavy-builds)
for policy, budget, exact-tree transfer, results and failure behavior. The default
allowlist is empty. Boat is a local prerequisite, not an injected VM credential.

In the Eon repository, run this crate's tests through Eon's test wrapper (see
Eon's [development guide](https://github.com/Yazelix/eon/blob/edge/docs/DEVELOPMENT.md)),
and run `cargo fmt --check` and `cargo clippy --locked -p rota-cli --all-targets -- -D warnings`
inside the pinned development shell before review. The slot suites that need a
route, Boat's among them, live in Eon's `eon-boat` crate.

## Retiring build output

`rota retire NAME --project P --bead ID` frees finished agents' Cargo caches.
Configure `[projects.P]` with absolute `repo` and `agents_root`. Names identify
`NAME-target` and `NAME-work/target` directly under that root. Target contents
are removed only from plain directories with their own regular, non-symlink
Cargo `CACHEDIR.TAG`. An empty directory at either exact target path can be
finished without a tag using `rmdir`; non-empty untagged targets stay.
The worktree and work directory stay. Every regular work file survives except
contents of that direct tagged Cargo target. Full retire also removes store
symlinks beneath the work directory and the exact root files `NAME-prompt.md`
and `NAME-launch.sh`. Symlinks to other locations stay.

Full retire requires `[projects.P.done]`: an argv `command` (with `{bead}` and
optional `{repo}`/`{name}`), JSON `pointers`, and string `equals`. The command
runs in `repo`, with a 30-second timeout and 1 MiB output limit. The first
resolving pointer determines done status. See `config.example.toml` for a Beads
check. A sole configured project needs no `--project`.
When supplying `--bead`, the caller vouches that NAME and the bead identify the
same finished task.

`[cleanup] min_idle` defaults to `"2h"`; supported units are seconds, `s`, `m`
and `h`, up to 30 days. Targets with any recent or future modification time
stay unless the done check passes. Empty target directories can be finished
without waiting for `min_idle`; live-session and full-retire done guards still
apply. `store_prefixes` defaults to empty, keeping
all work symlinks; configured prefixes match absolute link text by whole path
components. Parent traversal in a link is refused.

`--target-only` leaves prompts, launchers and store links alone and permits
cleanup without a done bead. It retains the full live-session guard: any owned
process using the worktree, work directory or targets (cwd, executable,
`CARGO_TARGET_DIR`, open file descriptors or mapped files), or a matching
unfinished registry record whose PID exists, keeps the session. An inspection
failure, including unreadable process details, keeps files and exits 1. A failed
explicitly requested done check also exits 1.
The guard checks resolved paths plus cwd ancestor device/inode identities.
Bind-mount aliases of agent directories are unsupported and may escape
detection; do not rely on the guard to protect such a setup. `rota retire --help`
also describes this limit. Unreadable fd or maps details for an owned process
refuse cleanup with its PID named; other users' processes are skipped.
Run retire after stopping work for the session; a scan cannot prevent a later
build from starting.

`--dry-run` performs the guards without changing files or permissions. `--json`
reports each path's kind, disk bytes, outcome and reason, plus `freed_bytes`
(zero in dry-run). Exit codes are 0 for completed cleanup or nothing found,
3 for a policy keep, 2 for input/config errors and 1 for inspection/removal
errors. Partial removals stop at the first failure.

Readable read-only targets, including the top-level directory, are made writable
during cleanup. Interrupted cleanup can be retried after min_idle or with a
passing done check, under the usual guards, including after tag deletion when
only the empty directory remains.
Retire requires Linux 5.6 or newer.

## Design principles

- **Choose across backends, not across accounts.** rota chooses between
  providers. Several accounts of one provider are only a config option, used
  at your own responsibility under each provider's terms.
- **Concurrency caps per backend.** A backend can allow a set number of
  sessions at a time, for example one on a small plan.
- **Refuse rather than fall back silently.** When no backend fits, rota says
  so and exits. A fallback happens only when the config names one, and rota
  reports it.
- **Require the command guard.** rota refuses a backend whose configured
  command guard (for example [dcg](https://github.com/Dicklesworthstone/destructive_command_guard))
  is missing.
- **Proxy-backed backends.** A backend can point its CLI's base URL at a local
  proxy such as CLIProxyAPI, so adopting a proxy is a config entry.

Personal details (paths, accounts, thresholds) live in your config file. The
repository will ship only an example config.

## Related tools and how rota differs

These tools solve nearby problems well, and rota borrows ideas from several of
them. [docs/references.md](docs/references.md) has the details.

- [caam](https://github.com/Dicklesworthstone/coding_agent_account_manager)
  switches between accounts of each coding CLI, ranks them by live usage, and
  can switch account after a rate limit. rota chooses between backends instead,
  and adds concurrency caps and a model or effort level per task.
- [ntm](https://github.com/Dicklesworthstone/ntm) runs many agents in tmux
  panes and can ask caam to choose an account before it starts a pane. rota is
  a single launcher that any orchestrator can call.
- [T3 Code](https://github.com/pingdotgg/t3code) is a GUI for coding agents
  that shows usage limits across providers, but by design does not route on
  them. rota routes, and borrows its quota-reading approach.
- [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) exposes
  subscription CLIs as an API and pools requests across accounts. rota works
  per session rather than per request, and can use the proxy as a backend.
- [claude-swap](https://github.com/realiti4/claude-swap) and
  [clauth](https://github.com/uwuclxdy/clauth) manage several Claude Code
  accounts. rota covers other providers too.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
