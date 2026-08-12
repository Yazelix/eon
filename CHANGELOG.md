# Changelog

This file records user-visible runtime, command, configuration, packaging,
installation, and proven contract changes.

## Unreleased

- Expose generation listing, exact attach, and owner-routed stop through
  `eonterm` so its separate runtime namespace remains manageable across updates.
  Reconnect guidance names the exact generation.
- Paste ordinary native clipboard text through Orbit's semantic normal or
  bracketed-paste path in packaged Eon and Eonova surfaces.
- Deliver bounded terminal-requested text to the ordinary or primary native
  clipboard in packaged Eon and Eonova surfaces.
- Render Unicode Braille progress cells with distinct dots in packaged native
  Eon and Eonova surfaces while preserving their cell-grid bounds.
- Preserve Shift-produced layout text such as `?`, `:`, and `+` in the packaged
  native terminal surface without changing modified shortcuts.
- Component graph schema 3 removes inert artifact paths and launch declarations;
  Nix owns physical paths and Rust owns launch policy.
- Preserve native Fish per-command completions when Carapace is enabled while
  retaining its fallback definitions for commands without native coverage.
- Reject an invalid embedded component graph before selecting a runtime
  generation, creating runtime state, or starting a child.
- Serialize supervisor control-socket acquisition so concurrent launches
  converge on one live owner instead of unlinking a newly bound peer.
- Refuse a whole-generation stop when its validated supervisor is replaced
  during confirmation, preserving the replacement and its live Sessions.
- Resolve the four stable managed-shell commands from the Session-private PATH
  so direct package launches do not depend on an ambient Eon profile.
- Include generated managed-shell and private Session command behavior in the
  runtime-generation identity so profile updates select a fresh supervisor
  without stopping older work.
- Preserve every opaque Orbit endpoint byte in workspace-action JSON by
  reporting `endpoint` as an ordered integer array instead of a lossy string.
- Publish `eonterm [--no-decorations] -- COMMAND...` as the slim reusable
  terminal product. It preserves Eon's proved exact-command lifecycle while
  excluding the managed shell and tool bundle. Remove the former `eon terminal`
  spelling without an alias.
- Supply Symbols Nerd Font Mono to packaged Venus so Yazi can render its
  supported private-use icons without an ambient font.
- Exit quietly when a downstream reader closes piped CLI output while keeping
  other detected stdout write failures explicit.
- Isolate supervisors by a deterministic runtime-generation identity. Bare
  `eon` attaches only to the exact current generation while older live and
  fixed-namespace legacy workspaces remain discoverable. Add deterministic
  human and JSON generation listing, explicit compatible attach, and confirmed
  owner-routed whole-generation stop without PID or process-tree inference.
- Extend EONW v1 with additive supervisor identity and stop actions whose result
  type remains separate from the pinned Venus workspace response.
- Supply pinned Nushell, Bash, Zsh, Fish, Starship, Zoxide, Atuin, Carapace,
  Helix, Yazi, and LazyGit through bounded `eon-*` commands and a
  Session-private PATH. Let users select one direct shell argv and disable each
  shell integration in `config.toml`; load native shell and tool configuration;
  preserve existing prompt, completer, and same-tool hooks; keep Nushell user
  autoload last; honor `ATUIN_NOBIND`; and suppress Nushell's stock banner.
- Keep the user's global shell environment unchanged, resolve a relative Eon
  configuration root once, and reject ambient Helix, Yazi, or LazyGit
  configuration-path overrides that bypass Eon's private root.
- Add Eon-owned live tabs and vertical pane stacks with stable-ID and direct
  directional CLI actions, deterministic JSON output, and one Orbit process per pane.
- Remove a pane when its Orbit Session exits, select the nearest survivor,
  remove an empty tab, and close Eon when the final pane exits.
- Expose those actions and complete accepted workspace snapshots through the
  bounded, versioned, dependency-free EONW v1 local protocol while preserving
  human and JSON CLI projections.
- Render every fitting pane header around one selected Session, bind Alt+H/L and
  Alt+K/J traversal plus Alt+M and Ctrl+T creation, refresh external actions
  within one second, and keep reattachment available while another pane remains
  live.
- Keep terminal text and Unicode table borders on the authoritative Venus cell
  grid, and anchor shaped input-method preedit text and underlines at the cursor.
- Recover the selected live Session automatically after retryable local socket
  loss while retaining its last coherent presentation until a fresh frame.
- Ship the x86_64 Linux alpha through one locked Nix flake with an Eon desktop
  entry that groups its X11/Xwayland window and uses a transparent violet
  three-fold loop icon rendered at native launcher sizes.
- Add `eon`, `eon run -- COMMAND...`, `eon attach`, `eon versions`, and
  `eon config-path`.
- Make bare `eon` and the desktop launcher reconnect to the live workspace;
  keep `eon run` as the explicit start command.
- Supply pinned Mesa Vulkan drivers to desktop-launched Venus instead of
  depending on shell-only graphics environment variables.
- Keep Orbit sessions alive across Venus exit and reconnect them through the
  same private local socket.
- Ignore relative XDG base paths, create missing configuration and runtime
  directories with mode `0700`, preserve existing configuration-directory
  permissions, and reject unsafe existing runtime directories without mutation.
