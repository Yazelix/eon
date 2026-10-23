# Visual References

These discovery notes preserve visual ideas for later design work. They do not
select a dependency or asset, authorize source reuse, or create product scope.
Before implementation, route a named question through the owning repository's
reference and dependency gates at an exact revision.

- [System Atlas at `6b3c91b`](https://github.com/inkboard/system-atlas/tree/6b3c91bc54e638ff58699b04199d994369ba7e3c)
  is the preferred reference for a public interactive Eon explainer. Study its
  progressive chapters, inspectable flows, and one authored data model for the
  visual map. Present accepted user outcomes first and reserve Eon, Orbit, and
  Venus ownership for a final technical chapter. Keep the atlas as a
  release-stamped projection of canonical Eon docs and contracts; omit System
  Atlas's generated `SYSTEM.md`, decision table, and question IDs because Eon
  already owns that truth through contracts and Beads. Its MIT renderer remains
  comparison evidence until an exact source-reuse and toolchain decision; a
  public artifact also needs narrow-screen, keyboard, and reduced-motion proof.
- [Ramps](https://www.ramps.studio/) is the preferred starting point for a
  future Eon palette question. Study its perceptual OKLCH ramps, semantic
  light-and-dark tokens, enforced contrast, and machine-readable exports. Its
  [implementation](https://github.com/imryanreid/ramp-generator) is MIT-licensed;
  native and web clients can share chosen color values without adopting its web
  application.
- [Pryzm](https://pryzm.design/) demonstrates layered gradients, glass, grain,
  and restrained loops. Treat its live web-code export as inspiration rather
  than a Venus renderer choice.
- [Zoxilsi Studio](https://studio.zoxilsi.cc/) provides a compact playground for
  mesh topology, point colors, patterns, glow, and animation controls. Use it
  for visual exploration, not implementation authority.
- [React Bits](https://reactbits.dev/) catalogs motion and background treatments
  that may inform a future Eon web client. It does not select React or apply to
  Venus. Its current
  [MIT plus Commons Clause license](https://github.com/DavidHDev/react-bits/blob/main/LICENSE.md)
  restricts component redistribution; inspect the exact source and license
  before any reuse.
- [T3 Chat](https://t3.chat/) informs how fast switching between agents must
  feel, what a sidebar row shows while an agent works, and how one call
  launches a fully configured agent. Study its fast local switching as
  described in 2025 (client-side routing, preloaded messages, re-rendering
  only the streaming block), its sidebar spinner on threads still generating,
  and its `/new?q=&model=&profile=` launch URL. Its open request for a
  completion signal shows the gap a spinner alone leaves, and its branches,
  each a new top-level chat, show how forks clutter a list. T3 Chat is
  proprietary; the MIT-licensed
  [T3 Code](https://github.com/pingdotgg/t3code) and cloneathon starter are
  separate projects.
- The [ChatGPT desktop app](https://learn.chatgpt.com/docs/pets) for macOS and
  Windows informs Eon's attention states, switching, notifications, and focus
  preservation. Study its four ranked states (needs input, blocked, ready with
  unread activity, running), its Activity view of unread, running, and waiting
  chats, its "next chat needing attention" key in Codex mode, and its
  completion alerts set to never, background only, or always, with separate
  toggles for permission requests. Its floating Option+Space window shows the
  focus risk: in July 2026 a macOS user
  [reported](https://community.openai.com/t/companion-window-no-longer-returns-focus-to-the-previous-app/1388206)
  that closing it no longer returned focus to the previous app, and OpenAI
  support agreed that it should. The app is proprietary and its docs change
  often; recheck them before relying on a detail.

Backgrounds Supply, Grainient, Ditther, and Gradientora were not retained. They
mainly offer finished assets or image-editing workflows; revisit them only for
a chosen marketing-asset task.

Sources and access dates for the T3 Chat and ChatGPT entries are on
`eon-t3chat-chatgpt-desktop-refs-0ooo`.
