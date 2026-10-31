# Agent Guidelines for `runtime/`

These directory-local rules add to the repository root `AGENTS.md`, which
holds the protocol import and the repository-wide rules. Do not edit this
generated file directly. Edit `.agent-protocols.local.md` in this directory,
then render from the pinned source.

Eon Runtime owns the `eon-runtime` library and the canonical
`eon-workspace-protocol` crate (EONW).

Runtime modules stay private; consumers call `run(Inputs)` through the concrete
root exports. Eon supplies the product version, defaults, validated component
facts, launch paths, and immutable assembly inputs. Runtime owns configuration
parsing, lifecycle, workspace state, and generations.

From v1.0.0, keep the EONW lifecycle codec of every earlier 1.x release so its
generations stay discoverable and stoppable (EON-C28).

Do not add product defaults, a component-graph parser, Eon source includes, a
binary host, daemon, framework, platform promise, or release channel without
user direction.
