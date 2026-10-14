# Venus

Venus is Eon's native graphical client, `yazelix-venus`. It materializes
Orbit-authored presentation and Eon-authored workspace topology, collects
native input, and sends semantic actions and resize requests. The root
`AGENTS.md` applies; these rules add Venus's local limits.

## Boundary

Venus must not own a PTY, parse raw PTY output, instantiate another terminal
emulator, answer terminal queries, reconstruct hidden terminal state, or keep a
second protocol schema. Report an insufficient Orbit contract to Orbit instead
of compensating in the client.

One native window per Venus process shows either one standalone Orbit session
or an Eon workspace: horizontal tabs and accordion pane headers around exactly
one expanded Orbit surface, with Eon-authored popups, pickers, Eon Bar,
shortcut viewer, hints, version label, and quota chips. Typography, geometry,
opacity, blur, and cursor trail are startup inputs; Eon owns persistent
configuration. Arbitrary split trees, simultaneous expanded panes, sidebars,
settings, configuration files, remote or web access, plugins, shell
integration, compatibility windows, packaging, and distribution are outside
Venus. A new Venus module is product scope.

## Rendering and tests

Keep one deterministic path from canonical Orbit frames to Venus-owned draw
inputs. A headless backend or recorded corpus may prove decoding, composition,
damage, ordering, and failure behavior; it does not prove visual quality and
never becomes another product backend. Do not mirror the wire schema by hand or
claim that a lossy projection proves rich-presentation fidelity. Prefer golden
state transitions and property checks for deterministic composition, then
focused native dogfood for window behavior, text quality, input,
accessibility, and lifecycle. Keep disposable proof state, sockets, and
captures under one `mktemp -d` root cleaned on exit.

## Native host boundary

Native Linux Wayland with Vulkan is the proved target; `aarch64-darwin` with
Metal is the only approved expansion. Keep one shared protocol validator,
model, scene, draw-input, renderer, accessibility, and semantic-action path.
Isolate event-loop, window, graphics-surface, clipboard, IME,
accessibility-host, shortcut, and application-lifecycle mechanics at the native
host boundary. Add no generic portability framework, duplicate renderer,
protocol copy, X11 backend, Intel macOS target, signing, notarization, or
distribution surface.

## References and crates

When the user names a public project, or one materially informs architecture,
inspect its code at an exact release or commit as required evidence and
implement Venus-owned code unless the user approves another reuse boundary.
`docs/CRATES.md` records accepted crate decisions, including their Wayland,
input-method, accessibility, and Darwin consequences.
