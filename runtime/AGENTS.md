# Eon Runtime

Eon Runtime owns the `eon-runtime` library and the canonical
`eon-workspace-protocol` crate (EONW). The root `AGENTS.md` applies; these
rules add Runtime's local limits.

Runtime modules stay private; consumers call `run(Inputs)` through the concrete
root exports. Eon supplies the product version, defaults, validated component
facts, launch paths, and immutable assembly inputs. Runtime owns configuration
parsing, lifecycle, workspace state, and generations. Orbit owns terminals and
terminal state; Venus owns native presentation.

Preserve historical EONW revisions so older generations stay discoverable and
stoppable.

Do not add product defaults, a component-graph parser, Eon source includes, a
binary host, daemon, framework, platform promise, or release channel without
user direction.
