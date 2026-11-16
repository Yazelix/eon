# Security policy

## Report a vulnerability privately

Use [GitHub private vulnerability reporting for Yazelix/eon](https://github.com/Yazelix/eon/security/advisories/new).
You can also open the repository's Security page and choose **Report a vulnerability**.
Keep vulnerability details and exploit instructions out of public issues and pull requests.

We assess reports against the latest Eon release. The supported platform is
x86_64 Linux on native Wayland. Apple Silicon macOS (`aarch64-darwin`) is a
Nix-only preview, with the limitations recorded in
[EON-C7](docs/CONTRACTS.md#eon-c7--native-linux-wayland-and-apple-silicon-macos).
Reports about that preview are welcome; other platforms have no support commitment.

Include the Eon version or commit, installation channel, platform, relevant
configuration and agent sandbox settings, reproduction steps or a small proof
of concept, and the expected and observed behavior. Explain the impact and
what access an attacker needs. Remove credentials and private terminal contents
from logs and examples.

Maintainers triage the report in the private advisory, may ask for more detail,
and work with the reporter on a fix or mitigation and coordinated disclosure.
We publish confirmed issues and available fixes or mitigations through GitHub
security advisories and release notes. We make no response-time guarantee.

## Agent security during the beta

Eon defaults `[agents] terminal_access` to `"window"`, which permits reads and
input across panes and popups in every session of that window for same-user
clients with socket access. We recommend `"none"` when you do not run terminal
orchestration. It refuses direct terminal-content commands; the window's native
Scrollback shortcut remains available.

Sandboxed agents must not reach Eon's socket or helper sockets: a client that
can reach them can run commands on the host, even with `terminal_access = "none"`.
Helper sockets include Editor RPC (such as a Neovim server) and the Quick-search listener.
Verify these restrictions in the agent's sandbox. Eon does not isolate unrestricted
programs running as the same user.

The beta retains accepted risks: agents with terminal-input access can send
instructions or commands to other agents and shell panes, and launched agents
can inherit their launcher's credentials. Per-agent authorization for Eon's
control and helper sockets is tracked for work after the beta. Keep credentials scoped
to the task and treat terminal text, titles and attention messages as untrusted data.

See the [security model](docs/SECURITY-MODEL.md) for the threat model, known
limitations and deferred work, and [configuration guidance](docs/CONFIGURATION.md#terminal-orchestration)
when choosing terminal access.
