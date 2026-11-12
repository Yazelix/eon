# Managed-agent CLI, JSON and configuration

**Planned under [EON-C40](CONTRACTS.md#eon-c40--managed-agent-admission-and-execution).**
The managed-agent commands and launch settings below are accepted intent,
not implemented behavior. Existing terminal-access/send settings retain
their current contracts. [Usage](USAGE.md) and [Configuration](CONFIGURATION.md)
describe delivered behavior; each implementation slice updates them as it lands.

Approved on 2026-11-12 by Lucca's "you decide" over eon-desk's yes, relayed by
eon-overseer and recorded on `eon-eon-agent-lr8sx.22` in comment 7406.
The original specification was approved on 2026-11-07. The v0.2.0-cut hold
was removed on 2026-11-12; actual prerequisites and contract approval still
gate implementation.

## Command grammar

```text
eon agent launch [--brief FILE] [--name NAME] [--bead ID]
    [--role implement|review] [--level medium|high] [--only BACKEND]
    [--cwd DIR] [--pane] [--dry-run] [--json]
eon agent run --brief FILE --name NAME --timeout DURATION
    [--bead ID] [--role implement|review] [--only BACKEND]
    [--cwd DIR] [--effort EFFORT] [--add-dir DIR]... [--json]
eon agent ps [--all] [--json]
eon agent status [--cwd DIR] [--json]
```

Every subcommand accepts `--help` and `-h`; top-level `eon --help` lists agent.
No `eon rota`, rota aliases, old configuration import, or new daemon is added.

- `launch` defaults: current terminal, implement, high, canonical current cwd,
  no restriction to a backend, no prompt, and generated attempt name `agent`.
  A supplied brief must be a readable regular UTF-8 file without NUL, at most
  32 KiB; its full content substitutes one `{prompt}` argument. With no brief,
  that whole argument is omitted. An empty supplied brief remains one empty
  argument. Validate the brief and each candidate's expanded argv before
  admission: at most 256 arguments, each at most 32 KiB, and the encoded command
  within C10's 2 MiB frame bound. No shell/interpolation/prompt truncation;
  an OS argument/environment-size failure is still agent-start, never success.
- Without `--pane`, launch replaces itself with the chosen CLI, preserving
  interactive stdin/stdout/stderr. It creates an attempt record, but after exec
  cannot promise an exit status record: later inspection may know only that the
  exact process ended. `--json` is allowed here only with `--dry-run`; otherwise
  exit 2 before any launch. Provider output cannot be mixed with machine JSON.
- `launch --pane` is exactly the convenience composition `eon pane create
  --no-focus -- eon agent launch <same launch arguments without --pane and
  --json>`, in the caller's tab under C10's non-focusing placement, empty-tab
  and refusal rules. It retains the session's active tab,
  selection and shown popup, with C10's first-pane exception in an empty tab.
  There are no other placement flags. With --pane, --json formats the outer
  C10 creation receipt and is not forwarded to the interactive child; the
  receipt proves pane creation only, not backend admission/readiness. --dry-run
  and --pane together are invalid (exit 2, no pane). Before forwarding, the outer
  command canonicalizes --brief and --cwd against the caller's cwd, including
  canonicalizing and explicitly forwarding the caller's default cwd when
  --cwd was omitted. Relative paths therefore retain their caller meaning,
  not the destination tab directory. Failure to resolve a supplied path is
  reported before pane creation. This does not change the tab directory. Inner failure is the
  explicit-command pane's failure under C10, not a reason to retry creation.
- CLI `launch` starts a fresh managed agent attempt. There is no `--popup`
  launch flag in this draft: Agent `auto` is its existing UI entry, using the
  same picker. An exact background popup-show command belongs to the existing
  popup-parity bead, not a duplicate picker-specific popup API.
- `--dry-run` runs read-only admission probes and shows eligibility/pick and
  the executable basename, never arbitrary argv or environment values. It creates no lock,
  state, decision log, registry, cache or run directory; runs no
  preparation/refresh/trust hooks. It reports advisory counts: another start
  can change the pick before real admission. Existing quota state/schedule may
  be read, but dry-run performs no native refresh or shared-schedule write.
- `run` defaults to implement and canonical current cwd. It always requires a
  readable regular brief of at most 1 MiB, name and timeout; it has no fallback
  or interactive `--level`. The larger brief travels on stdin, never one argv
  argument; a stdin=null command names its brief file.
  `--effort` selects a configured backend run effort, defaulting to that
  backend's configured default. It uses a private per-attempt EON_RUNTIME_DIR after
  removing inherited Eon controls, and backend/run env cannot override that
  root. This prevents accidental default-window access, not a same-user
  security boundary. An unsupported `--only`, role, effort or
  add-dir shape is invalid input. It selects only entries with an applicable
  run command. `--add-dir` extends provider argv only; it cannot defeat the
  provider's sandbox. Stdin is the brief file or null, never inherited.
- DURATION is positive integer seconds, or integer `s`, `m`, `h`, maximum
  2,592,000 seconds (30 days). Deadline starts when the child starts, not when
  quota probing starts. Parent/child/group startup and cleanup are bounded.
  Normal exit, timeout or interruption all clean up the owned process group.
  Handle SIGINT, SIGTERM and SIGHUP. Signal the process group only while the
  leader is unreaped or that same group is proven non-empty; do not signal a
  potentially recycled group ID. Any descendant leaving the group may survive.
  If the runner crashes or receives SIGKILL, no cleanup occurs. Confirmed
  cleanup means the owned child/group only, never all descendants.
- NAME uses 1–64 ASCII letters, digits, `.`, `_`, `-`, not starting with `.` or
  `-`; attempt IDs are generated opaque identifiers, not NAME, pane IDs or
  Beads IDs. `--bead` is optional nonempty bounded metadata (128 UTF-8 bytes,
  no control characters); it never invokes br or changes work state.
- `ps` lists unfinished managed attempts by start time then ID; `--all` adds
  finished records. It includes launch, Agent-auto and run attempts,
  never arbitrary processes inferred as managed attempts. Cap counts do include
  matching unmanaged processes. No registry is an empty successful list.
- `status` reports each backend's enabled state, probe/guard/account/quota/cap
  evidence, and separate launch/run availability for each role. It writes no
  Eon files, runs no refresh/login/model turn, and needs no window. Native
  quota uses retained state only; custom probes must be read-only. Real launch
  may obtain a bounded on-demand native reading if no usable one exists;
  Claude shares C27's schedule, Codex keeps its per-collector limits.

Admission locking: launch/popup uses an immediate cross-process try-lock;
busy means exit 1 agent-busy. Headless run waits at most
agents.probe_timeout_seconds (default 20), then the same refusal. The lock
covers pick, probes, cap scan/recheck and spawn/exec; release on failure or
successful spawn/exec. There are no persisted reservations. Independently
started CLIs can race the scan; Eon serializes only its own admissions.
The probe setting bounds lock acquisition and the configured guard/account
checks that use it; quota readers keep their own deadlines. Capacity is
evaluated once after lock acquisition. No capacity queue or run --wait is
supplied, and the run's execution timeout still begins at spawn.

## Exit status and error shape

Keep Eon meanings, rather than rota's conflicting `3 = refusal`.

| Status | Meaning |
| --- | --- |
| 0 | Query succeeds (including no available backend/empty ps), dry-run has a pick, --pane's C10 creation succeeds, or headless exit 0 has confirmed owned-group cleanup. Never task/bead success. |
| 1 | Operational refusal/failure: no eligible backend, guard/account/quota/cap failure, busy admission lock, unsupported platform mechanism, failed spawn, unreadable records, output/write failure. |
| 2 | Invalid CLI/config/brief/name/timeout, invalid backend or role/mode/effort combination. |
| 3 | Only --pane forwards C10 missing/stale target or unknown-caller results; standalone admission never uses 3 for refusal. |
| 4 | Only --pane forwards C10's unavailable supervisor result; standalone exec/run/ps/status need no supervisor. |
| 5 | Reserved existing EON-C36 human-typing refusal; these new commands do not send input and never manufacture it. |
| 124 | Headless deadline expired, owned-group cleanup confirmed. |
| 125 | Headless owned child/group cleanup cannot be confirmed; final result records cleanup as unknown. |
| 130 | Interrupted; for run, only after bounded cleanup (125 takes precedence). |

After a headless child starts, pass through its exit status or `128+signal`
when cleanup and final recording succeed; record distinguishes provider exit
1/2/3/124/125/130 from Eon's failure. Missing final-record write returns 1
unless cleanup is unknown, in which case 125 takes precedence. Exec passes
provider status naturally. --pane returns creation, not backend admission.

Cutover: scripts branching on rota refusal 3 must switch to Eon refusal 1
and error.code. Do not translate all 3s: a started backend may itself exit 3.
The run's started receipt/outcome separates provider exit from pre-start error.

Use the existing failure object:
`{"error":{"code":"agent-unavailable","detail":"..."}}`.
New stable refusal codes: `agent-unavailable`, `agent-busy`,
`agent-cap-unreadable`, `agent-guard-failed`, `agent-account-failed`,
`agent-quota-unavailable`, `agent-start`, `agent-state`, `agent-unsupported`,
`invalid-agent`. Backend diagnosis uses structured reason codes rather than
requiring scripts to parse `detail`. Existing target errors retain their names.
If an effect may have started, include an optional `attempt` object in the
error response naming known effects and `outcome: "unknown"`; never report a
clean pre-launch refusal after an unacknowledged possible start.

Successful new JSON documents carry `schema: 1`; unknown additional keys can be
ignored. `ps` prints `{"schema":1,"attempts":[...],"problems":[...]}`;
`status` prints `{"schema":1,"backends":[...]}`. A damaged record appears in
`problems`, remaining records are printed, and ps exits 1. No raw config/env,
credential identity, tokens, full commands with secrets, probe output, brief
contents or provider transcript are included. Paths follow EON-C10's UTF-8
string-or-byte-array representation; times are Unix milliseconds.

`launch --dry-run --json` returns `schema`, `dry_run: true`, `backend` (or
null), `fallback`, role, level, cwd and per-backend `eligible`/`reasons` plus
warnings and executable basename. No pick exits 1 agent-unavailable. Missing
native quota is explicitly unavailable, but does not exclude built-ins that
allow_unknown_quota; no dry-run refresh is performed.

`launch --pane --json` returns the existing C10 pane-create JSON, with no
new agent-specific receipt schema. `run --json` emits one JSONL `started` receipt after durable
registration, then one `finished` result (or an error). Agent output is in the
private log only. Before registration there is one error object. Closing stdout
does not abandon cleanup or final recording of a live child.

An attempt exposes these required keys (nullable keys remain present):

```json
{
  "id": "opaque-attempt-id", "name": "example", "kind": "exec",
  "backend": "codex", "role": "implement", "level": "high", "effort": null,
  "bead": "example-123", "cwd": "/example/worktree", "brief": "/example/task.md",
  "fallback": false, "caller": "opaque-EON_TERMINAL-or-null",
  "window": null, "session": null, "tab": null, "pane": null,
  "popup": null, "terminal": null, "run": null,
  "pid": 42, "started_at": 1794050000000, "finished_at": null,
  "state": "running", "outcome": null, "exit_code": null,
  "log": null, "report": null, "cleanup_scope": null
}
```

`kind`: exec/popup/run. --pane's child registers an exec attempt. Optional
EON_TERMINAL is caller provenance, not permission to mutate a terminal or a
verified topology claim. Runtime can supply exact binding for Agent auto;
ordinary launch needs no supervisor lookup and leaves unavailable identities
null. `state`: starting/running/ended/unknown/finished.
`ended` means exact liveness ended but no authoritative exit result;
`finished` means a recorded terminal outcome; unknown inspection stays unknown.
`outcome`: null, exited, signaled, timeout, interrupted, failed, unknown.
Headless cleanup_scope is `process-group`; it is not a claim about escaped
children. A bare PID is display data only; private process-start identity or
Orbit identity supplies liveness. Unreadable liveness is not running or dead;
it stays unknown without making uncapped launch fail on an unproved platform.

Status backend objects contain name, enabled, running (integer or null),
quota (source, state, observed_at, windows, last_failure), and availability
by mode and role (`eligible`, `reasons`), shaped as
`availability: {launch: {implement: {...}, review: {...}}, popup: {implement: {...}}, run: {implement: {...}, review: {...}}`.
Each availability object is `{eligible, reasons, warnings}`; unavailable quota
accepted by policy appears as a warning, not an eligibility failure.
Each reason is `{code, detail, window, remaining_percent, resets_at}`, with
nullable window/value/reset facts. Codes are `disabled`, `role-not-allowed`,
`command-missing`, `executable-missing`, `guard-failed`, `account-failed`,
`excluded-root`, `cap-reached`, `cap-unreadable`, `quota-unavailable`,
`quota-limit`, `effort-unsupported`, `add-dir-unsupported`, or
`platform-unsupported`. No reason detail contains raw probe output. Every window has a stable key,
duration/scope when native, remaining_percent and nullable resets_at; native
keys are `300m` or `10080m/Fable`, with slash escaped as `%2F` and percent as
`%25` in scopes. `*` in limits applies to all returned windows. Quota state is `fresh`, `stale`, `blocked`, `unknown`, `missing` or `error`;
`last_failure` is null or `{at, reason}` with no raw reader output. Native
failure reasons retain EON-C27 names. A custom `fresh` reading means the
configured read-only probe succeeded now, not independent verification of the
upstream provider or its authentication. Source windows
are not identified by their current array index. Custom parsers keep their
named keys (five_hour, seven_day, weekly, etc.).

## `[agents]` configuration and defaults

New launching keys go in existing `[agents]` alongside unchanged
terminal_access/send_enter_delay_ms/send_typing_quiet_ms (EON-C35/C36).
No key is renamed and no singular table is introduced. One config; no ROTA_CONFIG.
Window-owned launch/popups use configuration resolved when that window opens
or reopens. EON-C11 presentation reload retains the managed-agent policy and
existing attempts; its presentation and quota-widget updates do not replace
that policy. Standalone exec/run/status use current config. Replies identify
the owning build/window, never combine the invoker's policy with another
build's supervisor. Managed-agent keys participate in EON-C9's existing
offline validation and TOML/JSON defaults, with matching optional omissions
and backend-array replacement. Configuration checking does not run probes,
inspect account availability, contact a supervisor or create state; it keeps
EON-C9's status 1 for validation failure and its unknown-key policy. Native quota reuses C27 without a cache/schedule redesign. For a
native source, neither backend env nor unset_env may name CODEX_HOME or
CLAUDE_CONFIG_DIR, even to set a default value. Opaque wrappers/account-selecting
commands use a custom reader. Inherited selection and credentials remain
C27/provider-owned: Eon checks declared inputs only, not credential/account
equivalence, arbitrary wrapper behavior, sign-out or switches at the same path.

| Key | Type / default | Meaning |
| --- | --- | --- |
| `agents.backends` | Array; Codex/Grok/OpenCode/Pi/Claude below | Whole supplied array replaces defaults; `[]` disables managed launching. Order is rotation order. |
| `agents.fallback` | Optional backend name; absent | Last resort only for interactive implement without --only. All declared checks/limits/caps apply. Never run/review. |
| `agents.probe_timeout_seconds` | Integer 1..300; 20 | Guard/account checks and maximum headless admission-lock wait; quota reader has its own deadline. |
| `agents.run.unset_env` | String array; `["DISPLAY", "WAYLAND_DISPLAY", "EON_*", "VENUS_*", "ORBIT_*"]` | Headless only; exact names or trailing wildcard; inherited session-marker scrub below happens first. |
| `agents.run.env` | String map; `{}` | Headless only, after unset, before backend env. Cannot restore removed session/desktop controls or override the private EON_RUNTIME_DIR. Not an OS sandbox. |
| `agents.backends[].name` | Required unique ASCII identifier | One backend/account policy entry; `[A-Za-z0-9_-]+`, at most 64 bytes. |
| `...enabled` | Boolean; true | False means no check, quota query, count or launch. |
| `...roles` | Distinct role array; `["implement"]` | Interactive eligibility; empty permits fallback-only entry. Headless roles come from run.commands. |
| `...commands.medium`, `...commands.high` | Both required for CLI interactive entries; absent for popup-only/run-only entries | Exactly one whole {prompt}; optional whole {cwd}. No shell/NUL/placeholder executable. Supplied brief excludes entries lacking these commands. |
| `...commands.popup` | Optional argv; absent uses high with prompt omitted | Agent auto's no-brief command; no prompt placeholder. Built-ins keep current launch/resume argv. |
| `...env` | String map; `{}` | Child/probe account environment; not printed. Values may be secret: private files preferred. |
| `...unset_env` | Exact-name array; `[]` | Remove inherited backend variables before env. Conflicting set/unset names fail config validation. |
| `...guard` | Optional argv; absent | If configured, successful read-only check required. Absence means no Eon guard requirement, never a claim of protection. |
| `...required_files` | Path array; `[]` | Required regular files; contents never read or printed by the picker. |
| `...account_check` | Optional argv; absent | Successful read-only check required if configured. Keep real account guard until equivalence is proved. |
| `...excluded_roots` | Path array; `[]` | Refuse canonical cwd inside a named root; unreadable configured root refuses account check. |
| `...allow_unknown_quota` | Boolean; false for custom entries, true for built-ins | Allows absent/unreadable/old quota, including failed reader, with a visible warning. Never overrides a valid supplied reading's limit failure. This deliberately broadens rota's narrower absent-reader-only meaning. |
| `...quota` | Optional table; absent | No reader is unavailable; allow_unknown_quota decides whether it blocks. |
| `...quota.source` | Required enum codex/claude/command | Native sources consume C27 facts and forbid declared set/unset of CODEX_HOME/CLAUDE_CONFIG_DIR, even defaults. Opaque account-selecting commands need a command reader. No equivalence verification. |
| `...quota.command` | Required argv for command source; absent otherwise | Read-only, bounded custom reader, backend account env applied. |
| `...quota.parser` | Required for command; agy/claude_text/claude_json/remaining_json | No Eon JSON parser or implicit custom auth adapter. |
| `...quota.agy_group` | String; `Claude and GPT models` for agy | Only accepted with agy parser; nonempty trimmed single-line group. |
| `...quota.timeout_seconds` | Integer 1..300; 30 for command | Native reader keeps EON-C27 10-second/64-KiB bounds, cadence and backoff; rejects this override. Custom output cap is fixed 1 MiB. |
| `...limits.implement`, `...limits.review` | Tables; default limits below for built-ins, empty for custom entries | Same configured role limits govern interactive and headless admission. |
| `...limits.ROLE.min_remaining` | Finite 0..100 percentage map; `{}` | Inclusive minimum; `*` is all returned windows. |
| `...limits.ROLE.max_used` | Finite 0..100 percentage map; `{}` | Exclusive used maximum, compared from validated remaining values. |
| `...limits.ROLE.strictly_above_min` | Boolean; false | Makes minimum exclusive. Threshold boundary math must not accidentally flip a decimal equality. |
| `...limits.ROLE.allow_missing` | Boolean; false | Permits absent named windows in a valid nonempty reading; unavailable-source policy is separate. |
| `...cap` | Optional table; absent, including built-ins | No implicit concurrency cap. Configured caps fail closed on unreadable/unsupported observation; built-in popup defaults do not require Linux observation on Mac. |
| `...cap.max` | Positive integer, at most 256; required when cap present | Same-user matching processes across roles, windows and unmanaged starts. No implicit unlimited fallback override. |
| `...cap.executable` | Required executable basename | Matched with resolved executable identity. |
| `...cap.runtime_executables` | Absolute path array; `[]` | Explicit wrapper/native identities when argv[0] changes. Do not ship private store paths. |
| `...cap.script` | Optional absolute path; absent | Distinguishes an interpreted CLI from other interpreter work. |
| `...cap.env` | String map; `{}` | Account filters, canonical directory aliases equal; not emitted in output. |
| `...cap.exclude_args` | Exact-argument array; `[]` | Exclude configured quota probes, not arbitrary substrings. |
| `...run` | Optional table; absent | No built-in headless command until configured/proved for a provider version. |
| `...run.commands.implement`, `...run.commands.review` | Optional argv; at least one required if run present | Headless mode eligibility independent of interactive roles. |
| `...run.stdin` | Required brief/null | Null requires command argument `{brief}`. Never inherit stdin. |
| `...run.efforts` | Nonempty distinct ASCII identifier array; required | Supported effort names for these commands, no universal model vocabulary. |
| `...run.default_effort` | Required member of efforts | Used if CLI omits effort. |
| `...run.add_dir` | Optional argv fragment; absent | Exactly one whole `{dir}`; repeated at whole `{add_dirs}` argument in command. |

Headless command arguments allow `{brief}`, `{cwd}`, `{report}`, `{effort}`
inside arguments in one left-to-right pass, never rescanning substituted data;
`{add_dirs}` must be a whole argument, never argv[0]. No general templating.
Config paths are absolute and literal; cwd/brief CLI paths may be relative and
are canonicalized. A placeholder needing UTF-8 refuses an unrepresentable path
rather than lossy conversion; opaque registry/target paths keep EON-C10 bytes.

Built-in entries (exact proposed defaults for config defaults/docs):

```toml
[agents]
probe_timeout_seconds = 20
# fallback omitted

[agents.run]
unset_env = ["DISPLAY", "WAYLAND_DISPLAY", "EON_*", "VENUS_*", "ORBIT_*"]
env = {}

[[agents.backends]]
name = "codex"
enabled = true
roles = ["implement"]
allow_unknown_quota = true
[agents.backends.commands]
medium = ["codex", "--no-daemon", "{prompt}"]
high = ["codex", "--no-daemon", "{prompt}"]
popup = ["codex", "resume", "--no-daemon"]
[agents.backends.quota]
source = "codex"
[agents.backends.limits.implement.min_remaining]
"*" = 0
[agents.backends.limits.implement]
strictly_above_min = true

[[agents.backends]]
name = "grok"
allow_unknown_quota = true
[agents.backends.commands]
popup = ["grok"]

[[agents.backends]]
name = "opencode"
allow_unknown_quota = true
[agents.backends.commands]
popup = ["opencode"]

[[agents.backends]]
name = "pi"
allow_unknown_quota = true
[agents.backends.commands]
popup = ["pi"]

[[agents.backends]]
name = "claude"
enabled = true
roles = ["implement"]
allow_unknown_quota = true
[agents.backends.commands]
medium = ["claude", "{prompt}"]
high = ["claude", "{prompt}"]
popup = ["claude", "--resume"]
[agents.backends.quota]
source = "claude"
[agents.backends.limits.implement.min_remaining]
"*" = 0
[agents.backends.limits.implement]
strictly_above_min = true
```

All omitted per-entry fields use the table defaults. A partial custom backend
array receives no inherited model/guard/limits/cap, so `eon config defaults`
must make the replacement-array rule explicit. Both levels use provider normal
model/effort defaults initially; they become distinct only through user argv.
Built-ins enforce positive headroom where usable quota exists and permit
unavailable quota otherwise, with a warning. Users with credits can explicitly
choose inclusive 0% as Lucca does. No built-in cap/guard or headless command.
Grok/OpenCode/Pi defaults are popup-only: their no-brief argv comes from today's
popup owner, not evidence of a prompt CLI format. Users configure
medium/high commands to make those entries eligible for CLI launch with briefs.
Review requires explicit role/run configuration; Eon ships no implicit crew.

Unknown keys follow EON-C9 warnings; known bad types, duplicate names, bad
references/argv/limits/paths and conflicting env fail before children. No
trusted_settings, prepare, refresh, state_file or log_file keys are accepted as
launch policy. Durable state uses `$XDG_STATE_HOME/eon/agents`, fallback
`~/.local/state/eon/agents`, and versioned records; no configurable duplicate
state graph. Directories 0700, files 0600; corrupt records are preserved.
`agent ps/status` do not create the directory. Headless log/report content is
private sensitive user output. There is no persistent decisions.jsonl in Eon:
admitted records retain backend/fallback metadata, refusals are returned only.
Existing rota history is left intact. Any resolved-config printer redacts env
values (backend/run/cap env alike); built-in config-default printing uses empty
maps and never reads the user's config. Secret-bearing argv is never printed.

## Child environment and resolved-config safety

Reuse managed_environment::remove_agent_session; do not invent a parallel
scrubber. The inherited names removed at the inspected revision are:

```text
CLAUDECODE CLAUDE_PID CLAUDE_EFFORT CLAUDE_CODE_CHILD_SESSION
CLAUDE_CODE_SESSION_ID CLAUDE_CODE_SESSION_ATTENDED CLAUDE_CODE_ENTRYPOINT
CLAUDE_CODE_EXECPATH CLAUDE_CODE_MESSAGING_SOCKET CLAUDE_CODE_MESSAGING_TOKEN
CLAUDE_CODE_SESSION_ACCESS_TOKEN CLAUDE_SESSION_INGRESS_TOKEN_FILE
CLAUDE_CODE_REMOTE_SESSION_ID CLAUDE_CODE_REMOTE_SESSION_UUID
CLAUDE_CODE_CLOUD_SESSION_ID CLAUDE_CODE_BRIDGE_SESSION_ID
CLAUDE_CODE_HOST_SESSION_ID CLAUDE_CODE_HOST_WORKTREE CLAUDE_CODE_HOST_WORKTREE_FENCE
AI_AGENT CODEX_SESSION_ID CODEX_THREAD_ID CODEX_TOOL_CALL_ID CODEX_VERSION
CODEX_PERMISSION_PROFILE CODEX_CI OPENCODE OPENCODE_SESSION_ID OPENCODE_TERMINAL
PI_CODING_AGENT PI_SESSION_ID PI_SESSION_FILE PI_REASONING_LEVEL GROK_SESSION_ID
EON_ATTACH_DELEGATED EON_ROUTED
```

Existing conditional tool-setting cleanup also remains: under CLAUDECODE=1,
remove only inherited GIT_EDITOR=true and COREPACK_ENABLE_AUTO_PIN=0; under
CODEX_CI=1 remove only NO_COLOR=1, PAGER=cat, GIT_PAGER=cat and GH_PAGER=cat.
Existing LANG/LC_CTYPE/LC_ALL restoration under Codex's C.UTF-8 values stays
with its current owner. This does not remove provider API keys or user account
configuration. Account variables are governed by the native-source declaration
rule above; credential identity is not inspected.

Headless then applies agents.run unset/env and backend unset/env; Runtime
finally enforces the private EON_RUNTIME_DIR and removed desktop/session
controls. Configuration may not recreate inherited session/credential-marker
names. These rules prevent accidental live-session access, not intentional
same-user execution. Resolved configuration printers redact every env value,
including run.env/backend.env/cap.env; key names may be shown. The command
printing built-in defaults reads no personal configuration.

## Quota and failure policy

Native real admission may request a bounded observation with chips hidden or
no window. Reuse C27's 10-second/64-KiB read, validation and existing schedules:
Claude's shared cadence/backoff and failure; Codex's per-supervisor/collector
cadence and in-memory failure. There is no cross-process Codex spacing/failure
guarantee and no new persistent schedule. Fresh means not old under C27: at
most one hour old, with only unexpired valid windows. Read-only status/dry-run
use retained facts only. No provider request merely to list defaults.

A valid not-old reading enforces every configured limit even if a later read
failed; the available failure is reported beside the retained facts. Claude's
shared provider_error does not itself prove usage exhaustion. Missing, old,
blocked, unknown, malformed or failed data is unavailable, never 0% or unlimited
capacity. Built-ins set allow_unknown_quota=true so unavailable data, including
Mac/API-key/failed-reader cases, does not prevent launching. Strict custom
entries default false and refuse unavailable data. A valid reading's limit
failure cannot be bypassed by this setting. Observed exhaustion expires with
C27's window/reset/freshness rules rather than becoming a permanent prohibition.
All failed limit windows and supplied resets are reported; credits/overage are
never inferred. No automatic refresh, login or model turn.

This unavailable-source policy is a new explicit product rule; rota's existing
allow_missing only skips absent named windows and did not ignore reader errors.
Guard/account/cap failures are separate and never become quota warnings.
