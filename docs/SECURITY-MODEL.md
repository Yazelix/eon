# Eon security model

This model describes Eon at `edge` revision `293f591e`
(`293f591eb0b51517f97a5585d7fabe1e694d4867`). It distinguishes implemented
checks, source-inferred weaknesses, and risks that need further verification.
The proved product platform is x86_64 Linux on native Wayland; the Darwin
implementation does not establish equivalent product proof
(`docs/AGENT-RULES.md:294`). Recommendations below describe future work, not
current guarantees.

## Beta policy update (2026-11-16)

The maintainer accepted cross-agent terminal input and agents inheriting their
launcher's credentials as beta risks. Agent authority across EONW and helper
IPC remains tracked for work after the beta (`eon-260mh.2`, `.3`, `.10`).
Sandboxed agents must not reach Eon's socket or helper sockets: a client that
can reach them can run commands on the host even under `none`. Helpers include
Editor RPC (such as a Neovim server) and the Quick-search listener.
These restrictions must be enforced by the agent's sandbox; the content
setting does not establish isolation. See the [public security policy](../SECURITY.md)
for private reporting and beta guidance, and the
[terminal orchestration recommendation](CONFIGURATION.md#terminal-orchestration)
when choosing terminal access.

## Assets and attackers

Eon brings people and agents into the same workspace. A window holds sessions;
a session holds tabs and panes; a pane shows a persistent terminal
(`docs/ARCHITECTURE.md:36`). The assets at risk are terminal contents and input,
repositories, agent execution authority, account credentials available to child
processes, local configuration and durable state, the desktop clipboard, and
the integrity of installed releases.

Threats enter through malicious repositories and files, prompt injection in
web pages or tool output, compromised agents, programs emitting hostile
terminal output, other local users, and tampered distribution artifacts.
Prompt injection needs a receiving agent to act on text; displaying text alone
does not establish command execution.

Eon's local Unix permissions separate users. Eon does not promise isolation
between processes running as the same user. Its terminal-access setting is a
preference for CLI content access, and an agent that can reach host control
sockets may exercise authority outside its own sandbox
(`docs/CONFIGURATION.md:939`, `docs/USAGE.md:1511`). Project directories, tabs
and windows are organizational boundaries, not separate OS security principals.

## Workspace control and terminal access

Eon Runtime creates runtime roots and generation directories with mode 0700
and rejects directories with a different owner or mode. It removes only an
owned stale control socket, binds its replacement as mode 0600, and validates
an endpoint's ownership, socket type and permissions on connection
(`runtime/crates/eon-runtime/src/supervisor.rs:142`,
`runtime/crates/eon-runtime/src/supervisor.rs:1017`,
`runtime/crates/eon-runtime/src/control.rs:187`,
`runtime/crates/eon-runtime/src/control.rs:605`). These checks restrict other
users; the EONW listener does not authenticate individual same-user callers
with peer credentials (`runtime/crates/eon-runtime/src/control.rs:653`).

`EON_TERMINAL` carries a terminal ID and run ID. Runtime requires an asserted
caller to match an exact live terminal owned by the workspace, but accepts
requests without a caller. This is routing and stale-caller validation, not
proof of the requesting process's identity. A same-user process can send a callerless request
(`runtime/crates/eon-runtime/src/sessions.rs:1772`,
`runtime/crates/eon-runtime/src/sessions.rs:1789`,
`runtime/crates/eon-runtime/src/cli.rs:2785`,
`runtime/crates/eon-runtime/src/supervisor.rs:2746`).

The product defaults to `[agents] terminal_access = "window"`, a 2000 ms
typing quiet interval and a 300 ms delay before Enter
(`eon/crates/eon/src/product.rs:122`). Content commands can address terminals
in shown or hidden sessions of the window. `none` rejects direct reads,
history exports, history editing and sends
(`runtime/crates/eon-runtime/src/supervisor.rs:4272`,
`runtime/crates/eon-runtime/src/supervisor.rs:4484`). It does not disable
workspace inspection or host command creation. Command-bearing pane and tab
creation require a live caller assertion, but that assertion supplies routing
rather than authenticated authority. Runtime starts the requested command in a
host terminal, so a sandboxed caller with EONW access can escape its execution
boundary even under `none`. This is the main host-execution risk (finding 5);
Editor and Quick-search sockets provide further routes (findings 13 and 18)
(`runtime/crates/eon-runtime/src/workspace.rs:1800`,
`runtime/crates/eon-runtime/src/workspace.rs:1986`,
`runtime/crates/eon-runtime/src/supervisor.rs:2911`,
`runtime/crates/eon-runtime/src/supervisor.rs:2927`). A snapshot details one
session and summarizes the others; it does not expose every terminal in every
session at once (`runtime/crates/eon-runtime/src/workspace.rs:588`). Explicit window
selection can reach other windows of the user (`docs/USAGE.md:1354`).

Post-audit update (2026-11-16): `eon-260mh.6` fixes finding 17's direct
Scrollback bypass; the reload/Orbit attachment route remains tracked in
`eon-260mh.2.5`. The description below records the audited revision.

There is a source-inferred exception to Scrollback denial under `none`.
A callerless EONW `InvokePopup` with `Toggle` skips the guard that covers
caller-bearing invocations and `Show`. With an eligible active tab and no
existing Scrollback popup, the subsequent path reads history, writes a private
snapshot and opens the configured editor. Exploitability remains
unverified; `none` must not be treated as blanket content denial
(`runtime/crates/eon-runtime/src/supervisor.rs:3973`,
`runtime/crates/eon-runtime/src/workspace.rs:1333`,
`runtime/crates/eon-runtime/src/supervisor.rs:4055`,
`runtime/crates/eon-runtime/src/supervisor.rs:4448`). The statement that
`none` reveals no terminal content in `docs/CONFIGURATION.md:944` overstates
the current protection; finding 17 is an exception to that published claim.

CLI paste accepts at most 64 KiB of UTF-8 and rejects control characters except
tab, LF and CR. Orbit also validates management paste and refuses multiline
paste when the program has not enabled bracketed paste
(`runtime/crates/eon-runtime/src/cli.rs:2163`,
`orbit/src/pty.rs:787`,
`runtime/crates/eon-runtime/src/sessions.rs:1657`). These checks prevent escape
sequence input through this path; they do not distinguish instructions from
data. A focused-target send requires verified native focus. `--force` sets the
typing quiet interval to zero. The send path adds no recipient-visible sender
provenance, consent step, security audit record or per-caller rate limit
(`runtime/crates/eon-runtime/src/supervisor.rs:4516`,
`runtime/crates/eon-runtime/src/supervisor.rs:4595`).

## Orbit, Venus and additional local IPC

Orbit requires an owned mode-0700 socket parent and sets its sockets to 0600.
Management lease acquisition checks the peer UID and expected identities.
In managed runs, interactive attachment requires the process ID and start
identity named by the lease holder. Unmanaged runs admit the first client.
These controls protect attachment; the management lease holder can still read
terminal text and send input, and same-user control of that holder remains
outside Orbit's isolation promise
(`orbit/src/socket.rs:87`, `orbit/src/management.rs:357`,
`orbit/src/host.rs:1050`, `orbit/docs/CONTRACTS.md:127`,
`orbit/docs/CONTRACTS.md:152`). Eon Runtime's upstream caller policy therefore
matters even when Orbit enforces its own admission rules.

The Editor popup has a separate IPC boundary. Eon adds a listener to compatible
packaged Helix and configured Neovim commands, under the private runtime
directory. Helix's open request carries paths as JSON data; Eon's Neovim
request invokes a constant Lua program with separate path arguments
(`runtime/crates/eon-runtime/src/managed_environment.rs:1864`,
`runtime/crates/eon-runtime/src/supervisor.rs:2912`,
`runtime/crates/eon-runtime/src/editor.rs:113`,
`runtime/crates/eon-runtime/src/editor.rs:355`,
`runtime/crates/eon-runtime/src/editor.rs:456`). That safe request construction
does not restrict a direct client's use of Neovim's execution-capable RPC
interface. Same-user socket access remains available under
`terminal_access = "none"` (`docs/CONTRACTS.md:2290`). A sandbox permitting that
socket can reach an editor outside the sandbox; this is a conditional high
risk when a Neovim listener is running. Exploitability of this source-inferred
path remains unverified. The packaged default editor is Helix
(`eon/crates/eon/src/product.rs:224`).

Quick search starts the pinned fzf 0.73.1 with `--listen` on a Unix socket
beside its terminal endpoint. Runtime sends an unauthenticated HTTP POST action,
`become(exit 11)`, to leave Quick search (`components/eon-v5.json:216`,
`flake.nix:839`, `runtime/crates/eon-runtime/src/pair.rs:231`). In that pin,
fzf treats a Unix listener as local, accepts execution actions from it, and
implements `become` by executing a shell command. See the pinned
[listener and authentication code](https://github.com/junegunn/fzf/blob/ce4bef75954bebd87e0886435bcf8c6904328ab0/src/server.go#L52),
[action dispatch](https://github.com/junegunn/fzf/blob/ce4bef75954bebd87e0886435bcf8c6904328ab0/src/terminal.go#L6511),
and [shell execution](https://github.com/junegunn/fzf/blob/ce4bef75954bebd87e0886435bcf8c6904328ab0/src/util/util_unix.go#L61).
Eon provisions no API key in this launch. An inherited `FZF_API_KEY`, if set,
makes fzf require a matching request header; Runtime's POST supplies none.
With a running listener, socket access and no required key (or possession of
that key), a client can send shell-execution actions outside its sandbox.
Finding 18 is therefore an established high conditional route to the same host
execution as finding 5, outside the terminal-content switch. The socket's
private directory and mode restrict other users, not permitted same-user
sandbox connections.

## Agent launch, credentials and context

Eon agent launch uses configured argv arrays with whole-argument substitution,
quota admission and a final process-cap recheck
(`eon/crates/eon-agent/src/launch.rs:307`,
`eon/crates/eon-agent/src/launch.rs:363`). Rota serializes its own launches;
independent backend processes bypass that admission, although subsequent scans
can count them (`eon/crates/eon-agent/README.md:201`). Resource caps do not
provide an execution sandbox.

The built-in fallback Agent popup commands omit approval-bypass flags and use
`codex resume --no-daemon`. When a rota configuration is available, the popup
uses that configured launch route instead
(`eon/crates/eon/src/product.rs:228`,
`eon/crates/eon/src/product.rs:257`,
`runtime/crates/eon-runtime/src/managed_environment.rs:2456`). The example
agent configuration contains agy permission-bypass and Codex
approval-and-sandbox-bypass flags.
These are example configuration choices, not flags imposed on every launch
(`eon/crates/eon-agent/config.example.toml:51`,
`eon/crates/eon-agent/config.example.toml:113`). The example Codex commands also
omit `--no-daemon`. Eon's usage guide records that a shared daemon can retain
another terminal's routing environment, including the authority to quit that
window without asking (`docs/USAGE.md:1518`,
`runtime/crates/eon-runtime/src/windows.rs:650`). Backend daemon behavior
requires version-specific validation; the example/default mismatch exists in
this revision.

Interactive launch inherits its process environment, applies backend
`unset_env` removals and then backend overrides. Headless `run` clears its
command environment, repopulates it from the inherited environment minus
`[run] unset_env`, applies `[run.env]`, backend removals and overrides, and
restores its manager connection variables. It does not construct a credential
allow-list or broker task-scoped credentials
(`eon/crates/eon-agent/src/readings.rs:15`,
`eon/crates/eon-agent/src/launch.rs:394`,
`eon/crates/eon-agent/src/run.rs:217`). Credentials present in that environment
can reach the backend; accessible home-directory credentials are a separate
filesystem exposure. The Boat client's removal of `SSH_AUTH_SOCK` applies to
its own client commands, not to all local launches
(`eon/crates/eon-agent/src/slots/boat.rs:292`).

When `trusted_settings` is configured, launch adds its working directory to
agy's `trustedWorkspaces` before executing the backend. The write is atomic;
there is no separate trust approval in this path. The downstream effect on
agy workspace configuration is **unverified**
(`eon/crates/eon-agent/src/launch.rs:95`,
`eon/crates/eon-agent/src/launch.rs:109`,
`eon/crates/eon-agent/src/launch.rs:363`).

Runtime strips listed agent-session markers and credentials, including
Claude Code's messaging socket and token variables, from children it starts.
It preserves intentional settings such as API keys. This prevents inheritance
of those listed session variables; it does not revoke credentials or block
same-user access to messaging services through other means
(`runtime/crates/eon-runtime/src/managed_environment.rs:2525`,
`runtime/crates/eon-runtime/src/managed_environment.rs:2640`).

The optional SessionStart hook in the usage guide runs human-readable
`eon session`. Its pane labels can contain sanitized program-authored titles.
Escaping controls does not remove natural-language prompt injection
(`docs/USAGE.md:1613`,
`runtime/crates/eon-runtime/src/pane_labels.rs:233`,
`runtime/crates/eon-runtime/src/workspace.rs:3506`). The packaged agent skill
tells agents to treat program titles, messages and terminal text as untrusted
data (`eon/skills/eon/SKILL.md:145`, `eon/skills/eon/SKILL.md:208`). That guidance
does not authenticate the text or enforce how an agent uses it.

## Terminal output and desktop effects

OSC 7501 records come from terminal output. Orbit accepts their reported app
and state, strips Unicode format characters from record text, sanitizes the
terminal title, bounds reply text and retains at most 256 records. Runtime
strips control characters again before exporting attention
(`orbit/src/status.rs:60`, `orbit/src/status.rs:165`,
`orbit/src/status.rs:217`, `orbit/src/status.rs:343`,
`orbit/crates/protocol/src/management.rs:20`,
`runtime/crates/eon-runtime/src/attention.rs:122`,
`runtime/crates/eon-runtime/src/attention.rs:893`). A hostile file printed into
a terminal can therefore supply a plausible app identity and attention message.
Treat these records as program reports, not authenticated permission requests.

Orbit captures OSC 52 writes only when capture is enabled. Each write must
contain one nonempty UTF-8 `text/plain` item without NUL. One PTY turn permits
at most 64 writes and 1 MiB total. Venus accepts the effect while attached,
writes the native clipboard without a consent prompt and reports the result
(`orbit/src/pty.rs:90`, `orbit/crates/protocol/src/session/mod.rs:34`,
`venus/src/model.rs:889`, `venus/src/application.rs:3497`,
`venus/src/application.rs:6992`). A program can replace text the user pastes
next. This revision has no user setting to deny or approve these terminal
clipboard writes.

Venus previews an escaped hyperlink target and validates opening as an ASCII
HTTP/HTTPS URI with a host and no credentials. It validates again before
spawning the desktop dispatcher (`venus/src/application.rs:250`,
`venus/src/application.rs:4803`, `venus/src/scene.rs:5257`,
`venus/src/application.rs:441`). The preview does not authenticate a website.
Runtime escapes notification body markup
(`runtime/crates/eon-runtime/src/notification.rs:301`).

Orbit forwards libghostty's terminal replies to the PTY. Existing real-PTY
coverage checks cursor-position, device-attribute and OSC 7501 probe replies
(`orbit/src/pty.rs:232`, `orbit/tests/real_pty/terminal_replies.rs:17`). Title
reporting and attacker-controlled reply payloads remain **unverified** in this
model. Missing coverage alone does not establish injection; legitimate color
query replies need not be suppressed.

## Filesystem and distribution

Runtime's private-file helper opens without following symlinks, validates an
owned mode-0600 regular file, and publishes through a checked temporary file.
Scrollback snapshots use new mode-0700 directories and mode-0600 files with
link-safe cleanup. Agent retirement's Linux directory traversal uses
`openat2` with `RESOLVE_NO_SYMLINKS` and `RESOLVE_NO_XDEV`
(`runtime/crates/eon-runtime/src/private_file.rs:39`,
`runtime/crates/eon-runtime/src/private_file.rs:123`,
`runtime/crates/eon-runtime/src/scrollback.rs:135`,
`runtime/crates/eon-runtime/src/scrollback.rs:242`,
`runtime/crates/eon-runtime/src/scrollback.rs:285`,
`eon/crates/eon-agent/src/cleanup/fs.rs:116`). These are checks on those paths,
not a claim that state, logs or worktrees withstand arbitrary same-user code.

Without `XDG_RUNTIME_DIR`, Linux uses a predictable product-and-UID path in
the temporary directory; Darwin uses `/tmp` with that naming scheme. The
directory validation rejects another owner's path. Where the parent is shared,
precreation can deny startup rather than grant control
(`runtime/crates/eon-runtime/src/platform.rs:46`,
`runtime/crates/eon-runtime/src/supervisor.rs:142`).

The package updater discovers releases from the fixed Eon repository. The
standalone installer accepts a pinned HTTPS archive URL and verifies its
checksum against a signed checksum list, using a separately provisioned signer
for identity `eon-release` in namespace `eon-bundle`. Downloads disable curlrc
and restrict protocols and redirects to HTTPS
(`eon/crates/eon/src/update.rs:32`, `installer/src/main.rs:355`,
`installer/src/main.rs:321`, `installer/src/main.rs:391`,
`installer/src/main.rs:426`). Users must verify the downloaded installer itself
before executing it, including a dry run. The documented bootstrap chain
checks the signature and installer digest first (`docs/INSTALLER.md:53`).

The direct updater requires a signer file outside its managed prefix,
requests interactive consent or `--yes` when consent is needed, and rechecks
the selected installation after consent. It checks that the signer file is
readable, regular and nonempty, but does not enforce a signer-file owner or
mode policy (`eon/crates/eon/src/update/direct.rs:256`,
`eon/crates/eon/src/update/direct.rs:480`,
`eon/crates/eon/src/update.rs:2208`). A changed trust anchor can authorize a
different signer; possession of a repository token alone does not establish
release-signing authority.

Nix composition declares a Cachix substituter and public key. The cache
workflow triggers on `edge` pushes or manual dispatch, restricts publishing to
the public upstream repository and needs its configured cache credential and
successful checks/builds before publication (`flake.nix:4`,
`.github/workflows/cachix.yml:3`, `.github/workflows/cachix.yml:18`,
`.github/workflows/cachix.yml:63`). Repository protections, actual workflow
activation, cache access policy and release private-key custody are
**unverified**.

## Conditional agent-spread scenario

A prompt-injected agent can use Eon's read/send paths if its tool permissions
and sandbox allow the CLI and socket connection. With window content access,
it can read another agent's prompt and paste instructions plus Enter. Eon adds
no trusted sender identity to that input. A receiving agent's interpretation
and tool permissions matter when the payload asks it to follow instructions
(`runtime/crates/eon-runtime/src/supervisor.rs:4484`,
`runtime/crates/eon-runtime/src/supervisor.rs:4595`).

Other input paths need no receiving agent to interpret hostile instructions:

- **Shell panes:** paste followed by Enter can execute a command at an idle
  shell prompt. Bracketed paste frames the text; the later Enter is a separate
  ordinary key event (`orbit/src/pty.rs:793`, `orbit/src/pty.rs:814`,
  `runtime/crates/eon-runtime/src/cli.rs:2206`).
- **Approval prompts:** an Enter-only send reaches the highlighted/default
  action of a TUI prompt. If that action approves execution, a sender can
  approve it without persuading the receiving agent. The selected default,
  whether Enter confirms it, and the resulting permissions depend on the
  backend and prompt state; this model does not assert that a bare Enter
  always chooses Yes (`runtime/crates/eon-runtime/src/cli.rs:2202`,
  `runtime/crates/eon-runtime/src/supervisor.rs:4598`).
- **SSH panes:** input sent to a terminal already running an authenticated SSH
  shell reaches that shell's remote input. A pasted command plus Enter at its
  prompt can execute on the remote machine with that session's authority.
  This uses an existing connection; it does not require Eon remote-access
  support or a receiving agent (`runtime/crates/eon-runtime/src/sessions.rs:1546`,
  `orbit/src/pty.rs:814`).

These paths require a suitable target program/state and successful input
delivery, including the content policy and typing checks. They establish a
high conditional execution risk without demonstrating universal or autonomous
worm propagation.

The reverse direction also matters. An orchestrator can read hostile program
output through `pane read`, treat it as an instruction, and then use its own
workspace or tool authority. Eon returns terminal text as data with metadata;
it does not make that text trustworthy. Sanitized titles, attention records
and session-start context offer additional entry points. The skill's untrusted
data guidance reduces ambiguity but cannot enforce the orchestrator's decisions
(`runtime/crates/eon-runtime/src/supervisor.rs:4564`,
`eon/skills/eon/SKILL.md:145`, `eon/skills/eon/SKILL.md:208`).

An agent allowed to create terminals can start configured or direct backend
commands outside its own sandbox, including under terminal access `none`.
Direct launches bypass rota admission. Unrestricted same-user execution can
also start processes without Eon. Persistence through user-writable
configuration and further remote access depend on available OS permissions
and credentials (`runtime/crates/eon-runtime/src/workspace.rs:1800`,
`docs/USAGE.md:1511`, `eon/crates/eon-agent/README.md:207`).

## Findings and recommendations

High means a path to substantial execution or credential exposure under the
stated conditions. Medium means a narrower policy, context or desktop-integrity
weakness. Low means limited availability or trust-anchor hardening. These are
qualitative priorities, not exploit scores. **Unverified / unrated** identifies
a question rather than an established vulnerability. The IDs identify each
finding across follow-up work; the referenced sections above supply evidence.

| ID | Severity / status | Finding and conditions | Recommended direction |
| ---: | --- | --- | --- |
| 1 | High, conditional | Read/send enables agent injection, direct shell commands, prompt activation and input to existing SSH shells; target state controls the outcome. | Define authorized sender/recipient relationships or user approval; expose provenance and bound/audit sends. |
| 2 | High, configuration-dependent | Example backend commands disable approvals or sandboxing when copied. | Ship sandboxed examples and require explicit opt-in for bypass configurations. |
| 3 | High, conditional | Backend environment inheritance can expose available credentials; home access can bypass environment restrictions. | Limit inherited credentials and pair that policy with filesystem isolation. |
| 4 | Medium, established design limitation | EONW caller identity supplies routing, with callerless access and no per-agent authentication. | Establish a capability/identity boundary before relying on per-agent policy. |
| 5 | High, conditional | EONW command creation escapes a caller sandbox into host execution, including under terminal access `none`; a live caller assertion is required. | Define and enforce agent authority for host execution; cover the additional routes in 13 and 18. |
| 6 | Unverified / unrated | Remote repository and cache protections may permit unauthorized distribution. | Verify protections and release/cache authority separately. |
| 7 | Medium, source-supported | OSC 7501 app and attention text are self-reported. | Expose provenance and distinguish reported identity from verified process observations. |
| 8 | Medium, source-supported | OSC 52 can replace the clipboard without consent. | Offer a configurable write policy with a useful preview/notice. |
| 9 | Medium, source-supported | The optional session-start hook introduces program-authored labels into agent context. | Minimize hook text and label untrusted fields. |
| 10 | Medium, configuration-dependent | Configured `trusted_settings` enrolls launch directories without a separate approval. | Require explicit scoped trust enrollment; verify downstream effects. |
| 11 | Unverified / unrated | Release private-key access and custody are unknown. | Verify custody; keep signing authority outside agent execution authority. |
| 12 | Low, conditional | Predictable fallback runtime paths permit precreation denial of service in shared temporary parents. | Use a private per-user parent or fail safely without one. |
| 13 | High, conditional | A reachable Neovim Editor RPC socket is an additional host-execution route under `none`, with the authority impact of 5. | Restrict sandbox access to auxiliary IPC or replace general RPC with a constrained boundary. |
| 14 | Unverified / unrated | Terminal reply injection has no demonstrated payload here. | Inspect the pinned parser and add targeted real-PTY coverage. |
| 15 | Low, source-supported | Signer-file validation lacks an owner/mode policy. | Define a safe trust-anchor ownership and writability policy. |
| 16 | Medium, configuration-dependent | Example Codex commands omit the default popup's daemon isolation flag; shared routing can target another window. | Align examples with `--no-daemon` and verify the supported backend version. |
| 17 | Medium, source-inferred | Callerless Scrollback `Toggle` bypasses the `none` guard under the eligibility conditions above. | Distinguish native-window authority from unauthenticated EONW requests and cover the bypass. |
| 18 | High, conditional | A reachable Quick-search fzf listener accepts shell-execution actions when its API-key check permits the request; an additional route to 5. | Constrain auxiliary IPC actions/access together with the agent authority design. |

### Current recommended use

Keep worker agents in a sandbox that denies EONW and auxiliary host sockets;
grant workspace control only where the task requires that authority. Verify
the backend's actual socket restrictions. Remove bypass flags from copied
examples and use Codex's per-terminal mode described above. Set terminal
access to `none` when content orchestration is unnecessary. The maintainer
retained the `window` default for the beta. The post-audit update above
records finding 17's fix; command creation, reload and helper sockets retain
their separate authority. These measures reduce exposure; they do not create
isolation between unrestricted same-user programs.

Give agents credentials scoped to the repository, actions and duration their
task needs. Existing backend `unset_env` and `env` settings can reduce ambient
environment exposure, but cannot hide credential files or revoke inherited
authority (`eon/crates/eon-agent/src/config.rs:174`,
`eon/crates/eon-agent/src/config.rs:203`). Treat terminal text, titles and
attention reports as untrusted data. Verify installer bytes before execution
and keep the independently trusted signer record outside the managed prefix.

### Proposed product work

An environment allow-list, per-role credential profiles, a mint/revoke broker,
and a filesystem boundary for agent processes are recommendations. Eon does
not implement them in this revision. Environment scoping alone cannot protect
credentials readable through the home directory. Any isolation design must
preserve the supported platform boundary rather than assume a particular user
service manager (`docs/AGENT-RULES.md:303`).

Per-agent authorization also needs an enforceable identity or capability
boundary. A secret placed in a same-user-readable environment, or peer UID
alone, is insufficient to isolate unrestricted same-user processes. Design
that boundary together with recipient consent/provenance and control of
auxiliary IPC. These proposals require separate contract approval and
implementation proof.
