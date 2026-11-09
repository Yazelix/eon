# Interaction

## Hyperlinks

Hover an explicit OSC 8 link, or an `http://` or `https://` URL printed as
plain text, to highlight it and preview its actual target.
Ctrl+left click opens it on Linux; Command+left click opens it on macOS.
The pointer becomes a hand while that modifier is held. Agent status updates
in the sidebar preserve hover and a held click on an unchanged link.
Ctrl+Shift+C on Linux or Command+C on macOS copies the hovered target
unless the terminal has selected text, in which case it copies that selection.
Both Command actions on a hovered link are observed natively on Apple Silicon
in this repository (`VEN-C16`).
Ordinary clicks retain terminal mouse reporting and selection behavior.
Plain-text detection joins soft-wrapped rows, drops trailing sentence
punctuation and unmatched closing brackets, and keeps only targets the opener
accepts; an explicit link on any of its cells wins. A loopback host with a
port, `localhost:3000`, `127.0.0.1:8080` or `[::1]:5173`, optionally followed
by a path, is detected too, and its target is `http://` followed by the text;
Eon's `[terminal] detect_local_hosts = false` (`--detect-local-hosts false`)
turns that off. Other schemes, other bare hosts, and file paths are not
detected. A URL that a program
breaks onto a new line itself, instead of letting the terminal wrap it, is
detected only up to that line end, so check the preview before opening it. The
accessibility tree exposes each current visible target as an Open link followed
by a Copy button. A changed frame retires both. Hover and actions pause during
scroll animation and renderer recovery.

Opening accepts ASCII HTTP/HTTPS targets up to 4096 bytes, with a host and
without credentials. Other schemes, malformed targets and oversized links
produce an accessible notice. Copy accepts target text up to the same limit
without control characters, including schemes that cannot be opened.
The native Linux host must provide `gio` on PATH and a registered HTTP/HTTPS
handler; macOS uses fixed `/usr/bin/open`. Venus passes one exact URI argument
without a shell, allows one dispatch at a time, and retires a stalled dispatcher
after ten seconds. Copy does not require an opener. Broader compositor and
fractional-scale proof remain open.

## Workspace and terminal interaction

The accepted Eon snapshot supplies the authoritative terminal endpoints: the
active tab's selected popup, otherwise the expanded Orbit pane of each of its
columns. Input goes to the popup or the selected pane. An empty body
detaches the presentation without stopping hidden terminals.
While the window is open, Venus subscribes to Eon's events (session, tab,
pane and the rest) and re-inspects Eon as soon as one arrives, so accepted
changes from another client appear without a click or restart; it also
re-inspects every 250 ms as a fallback, which alone keeps it current while
no subscription is open. A hidden popup stops showing as soon as its event
arrives, before that re-inspection answers.
Recovery continues only while that exact attachment remains current and live;
endpoint replacement or authoritative offline state cancels obsolete retry
state.

### Workspace controls

Click a tab or pane header to select it, or click another column's terminal
to select that column's expanded pane. The fixed controls in the Eon Bar are
`+` for a tab, `?` for Shortcuts, and `×` to close the exact active tab. Only
the empty region between tabs and controls drags an undecorated window.
While the snapshot lists two or more sessions, or its one session has a
name, an outlined label leads the tabs with the shown session's name or `sN`
and, with several, its `N/M` position. A click, or Enter or Space when it has
focus, invokes Eon's `sessions` popup entry as that entry's key does; without
that entry the label is inert text. A final close shows the close prompt
whatever the session count; while other sessions remain, it says it closes
the session, by name or `sN`, and its footer reads `Enter Close`.

| Keys | Action |
|---|---|
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Ctrl+Tab / Ctrl+Shift+Tab | Select the next or previous tab, wrapping around |
| Alt+H / Alt+L | Walk columns, continuing into the previous or next tab |
| Alt+K / Alt+J | Walk the selected column's panes |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the selected pane into the neighbouring column |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane within its column |
| Ctrl+Alt+Shift+H / Ctrl+Alt+Shift+L | Move the active tab |
| Alt+Shift+T | Request a pending tab |
| Alt+M | Create a pane in the selected column |
| Ctrl+Alt+M | Create a column right of the selected one |
| Alt+Shift+Z | Show the selected pane alone across the tab, or restore the columns |
| Alt+W | Close the selected pane; the final tab's last pane quits the window, or closes the session while others remain |
| Alt+Shift+W | Close the active tab; closing the final tab quits the window, or closes the session while others remain |
| Alt+Shift+N | Open an independent Eon window when supervised by Eon |
| Alt+Shift+D | Detach: hide the window, keep terminals running |
| Alt+Shift+Q (on macOS, also Command+Q) | Quit: close the window, stop its terminals; asks first while a terminal is busy, and is absent without Eon's running-windows summary |
| Alt+Z | Show or hide quick search, Eon's `project` popup entry; Eon supplies other popup shortcuts |
| Alt+/ | Open or close Shortcuts |
| Alt+Shift+B | Show or hide the bottom shortcut hints |
| Alt+Shift+H | Show or hide the agent sidebar, unless Eon turned it off |
| Alt+Q | Open or close the attention inbox under every sidebar setting |
| Alt+G / Alt+Shift+G | Send `NextAttention` or `PreviousAttention`, even with the sidebar off |

A compositor close request (the desktop's close-window key or the window's
close button) acts as Alt+Shift+Q, prompt included; another close request while
the quit prompt or the final-close prompt is open leaves that prompt to answer.
The quit prompt takes the place of an open attention inbox or Shortcuts viewer.
Without Eon's running-windows summary, while Eon is unreachable, or while it
refuses refreshes after a failed Stop, a close request detaches as Alt+Shift+D
does. A close request carries no reason, so a desktop that closes each window
at logout, such as KDE Plasma, gets the same quit; Alt+Shift+D before logging
out keeps the terminals there.

On macOS, Command+Q in an Eon window is such a close request, and
Shortcuts lists it beside Option+Shift+Q. Venus takes the key off winit's app
menu, whose Quit item stays and still only exits Venus, leaving the window's
terminals running. A standalone Orbit session keeps AppKit's Command+Q, which
exits Venus and leaves Orbit live.

In the attention inbox, Left/Right cycle All, Needs input, Error and Finished;
Up/Down highlight a row. Enter, Space or a row click sends that row's
`FocusAttention` and closes the dialog; the jump can show another session.
Escape or Alt+Q closes without a jump. Wheel, PageUp/PageDown, Home and End
scroll. The filters keep Eon's queue order and show whole-queue counts. The
inbox captures other input while open and restores prior keyboard focus on
dismissal. A popup configured on Alt+Q takes the fixed binding.

A zoomed tab's name is followed by a zoom-in magnifier (Nerd Font U+F00E),
with ` · zoom` as the missing-font fallback. Its accessible name keeps `, zoomed`.

Eon decides which actions are available and owns popup lifetime and directory
changes. A repeated press does not duplicate a structural action. Switching
tabs retains each tab's popup selection. F6 cycles terminal, agent sidebar,
tabs, and visible panes, skipping the sidebar while it is hidden or has no
rows and returning to the row it last focused while that row remains. Up/Down
traverse sidebar rows, and Enter or Space sends `FocusAttention` for the
focused one, as a click does; keyboard focus stays on the row until a snapshot
shows the exact target focused, then moves to the terminal, or to the tabs
when the jump's own accepted answer only selected the target's tab. A rejected
jump leaves focus on the row beside its notice. Keyboard focus returns to the
terminal when the sidebar closes or hides for width, or loses the focused
row. Arrow keys traverse focused tabs or panes, and Left/Right with tabs
focused walks columns before tabs like Alt+H/L; Tab and Shift+Tab traverse
header controls, the session label first while it opens the picker; Enter or
Space activates one. Focus on the label moves to the tabs when it hides or
turns inert. Escape returns chrome focus to
the terminal. With terminal focus, Escape, Ctrl+C, Tab, and Enter reach Orbit as
ordinary input.

Shortcuts shows fixed bindings and enabled popup entries. Wheel, arrow keys,
Page Up/Down, Home, and End scroll it. Closing with Escape, Alt+/ or a click
outside the dialog restores the previous focus without sending a terminal key,
pointer event or workspace action.
Alt+Shift+N also works while Shortcuts is open. A failed new-window request
appears as a local notice in the current window.

The bottom hints bar starts visible and reserves its own compact native row.
Alt+Shift+B returns that space to the terminal or restores the bar without
changing the selected tab, pane, popup or focus. The choice lasts for that
window, including across resize. Hints use the current bindings and enabled
popup labels, and follow native terminal, tab and pane focus. Each set of
modifiers collapses into one keycap labelled with their full names, in the
order the hints first use them: `Alt`, `Ctrl+Alt`, `Alt+Shift` and
`Ctrl+Alt+Shift` by default, with macOS's own names there. The keycaps form one
cluster centred on the window's centre line, one padding apart; keys without
modifiers follow inside the cluster, written out (`Escape terminal`, and the
arrows while the tabs, panes or sidebar have focus). While the attention inbox
is open, the cluster is only its keys. The clock sits at the bar's left end
and `EON <version>` at its right end, each one padding from the window edge
and both plain text in the version's softened cyan,
while `clock` and `version` are in `[bar] widgets`; they hide with the bar,
and their tooltips open above them. Neither takes focus or acts on a press.

Pointing at a keycap fills it with the chrome highlight and opens a popup above
it, inside Venus's own surface: its left edge at the keycap's, kept inside the
window, and its bottom a third of a padding above the bar. It has no title,
since the keycap below names its modifiers. Its hints sit in sections side by
side, one padding apart: `Actions` in the first column, then `Popups`, each
under a small title in the secondary text colour, each hint in bar order with
its keys in mint and a short label in white (`New tab`, `Close tab`, `Zoom
pane`, `New window`, `Detach`, `Hints`, `Sidebar` for Alt+Shift; the popups'
own labels). Actions follow the bar order, which runs tab, pane, window, then
the chrome toggles; custom popups follow the built-in ones. The popup is never
taller than the room between the header and the bar: a section with more rows
than fit continues in the next column under an empty title line, so rows line
up. When its columns are wider than the room (40 ems at most), the columns that
do not fit give way and the last one shown ends with `Alt+/ lists all`, which
takes a full column's last row, or its title when only one row sits under it.

The popup stays open while the pointer is over it or the gap below it, and the
chrome and terminal links beneath it then take no hover. Resting the pointer on
a row whose full help says more than its label (every action) opens a box a
third of a padding above the popup, as wide as it, with that sentence in white,
word-wrapped; it shows only when the whole sentence fits below the header, so
very short windows show no box. The popup never takes keyboard focus
or changes terminal focus, and a press on a keycap or the version does nothing
else. Escape or a pointer press hides the popup until the pointer leaves that
keycap's hover band and the popup, as a tooltip does; the key or press still
goes where it would without the popup.

Narrow windows give way in this order, and keycaps never drop: the cluster
stays centred while it clears the clock and the version by one padding, then
shifts to sit one padding from whichever it would cross; when it no longer
fits between them, the version hides; when it no longer fits after the clock,
the clock hides and the cluster centres again; then keys without modifiers
drop whole, `Escape terminal` last; and when the full-name keycaps alone exceed
the bar, they switch to the short legends `A`, `CA`, `AS` and `CAS` (C for
Ctrl, A for Alt, S for Shift), which clip only in the narrowest windows. With
Eon's packaged font (DejaVu Sans) at the default size and the terminal
focused, 1280- and 800-pixel-wide windows show the clock, the centred keycaps
and the version; a 400-pixel window shows the full-name keycaps alone; a
330-pixel window shows the short legends. Each pane is its own
rounded card, stacked without gaps in its column's allocated card area. Even
outer gaps surround and separate the allocated column areas. Pane titles sit
in rounded edges, with neutral outlines sized by `border_width`. Collapsed rows'
corners turn toward the expanded pane, and their sides join the next card in
that direction.
Pane labels come from Runtime: the pane ID, then a pinned name, otherwise its
sanitized terminal title, otherwise its compact working directory. A differing
terminal directory adds ` · DIRECTORY`, using a relative or shorter compact path.
Labels keep explicit connecting, unavailable and offline states. The ID is dimmer
when the resolved background, including an extended header band, and opacity
permit readable contrast; narrow labels
keep the ID when it fits, while accessibility and close listings retain the full
bounded label. Metadata and names are display text, never command input.

The selected pane has a brighter label and a whole outline in
its focus colour, at the configured focus border width; hover adds a label pill and
F6 keyboard focus keeps a mint pill outline. Turning pane frames off leaves labels and
their interaction feedback. The hints sit bare in the margin below the cards. Hiding the footer returns its row to terminal content.

With pane frames on, each column has one continuous terminal-background fill,
from its first pane's rounded top edge down to the column's rounded bottom.
The last collapsed pane's label sits in that bottom edge, closing the stack with
one line. At the unscrolled bottom, columns and the Agents card share one snapped
painted bottom line. Its inset is half the title height, bounded by terminal
bottom padding minus the widest stroke, so frames stay clear of grid cells.
After resize rounding, the shared line moves outward as needed to leave a
full stroke beyond every visible terminal grid, within the allocated bottom.
A last expanded frame ends there inside its bottom padding when space permits.
When collapsed rows follow it, the expanded frame's painted bottom instead
sits one title row above the next label line. The same padding and rounded grid
bounds limit how far it rises; its rounded joints and the rows' sides meet
the raised edge. The column keeps its continuous background fill.
While a resize is pending, both edges also clear the retained frame's displayed
rows; the full rise returns when the resized frame arrives.
If a short, scrolled viewport cannot fit its bottom stroke below the grid, the
stroke stays below the grid and the card clips it. Terminal layout,
grids, header bands, hit targets and AccessKit bounds stay unchanged, as do
popup and frameless paint.
Last collapsed labels center on the line, clamped so their text boxes end at
or above the bottom of their unchanged title bands.
Stacked title rows and the space between cards share the expanded terminal's
background colour and opacity, except that extended top padding fills an
unzoomed expanded pane's band below its title (`padding_color`). Above that edge
and outside the outer corners, the gaps and footer stay the opaque bar colour.
The first title sits on that boundary. Popup and Agents fills follow their
rounded outer edges in the same way, with their titles on the boundary. Header
scrolling keeps the fill's top fixed. Frameless mode keeps terminal-background
strips across the window below the bar, including the sidebar and footer.

Keycaps are steel blue with light text. The compact tab range is 1..9; Alt+0
still selects the tenth tab. Merged keys share one popup row: the `Alt` popup
reads `HJKL · 1..9  Focus by direction, or a tab by position`, the `Ctrl+Alt`
popup `HJKL  Move pane between columns, up or down`, and the `Ctrl+Alt+Shift`
popup `HL  Move tab left or right`. A popup entry's row uses its configured
name. Alt+/, the Alt+Q inbox, Alt+Shift+Q quit, Copy/Paste, Open link, and the
Alt+G and Alt+Shift+G agent jumps stay in full Shortcuts, without footer hints.
Full Shortcuts and screen-reader labels retain complete chords and action
names.

The header `?` always shows; it and Alt+/ open the complete viewer. Hints are
read-only, including for screen readers: AccessKit reads them as one group,
"Eon shortcut hints", holding a label per keycap named by its full modifiers,
such as "Alt+Shift shortcuts", whose value names each section in turn
("Actions: ", then "Popups: ") before its chords and what each does as full
help says it, then one label for the expanded keys; the version label follows
the group.
They add no keyboard stops or actions. Eon has no minor modes yet.

### Labels and header scrolling

Popup margins shrink to leave a usable cell grid. Tab labels derive from Eon's
launch directories and retain stable `tN` identities. They preserve both ends
of a long path without cutting shaped clusters; hover wraps the full path.
Tab widths follow shaped text up to 280 logical pixels at default typography,
with the limit scaling for larger fonts.
Shell `cd` changes pane metadata, not the tab's launch directory. Pane headers
show pane identity and a home-relative or absolute working directory, keeping
rightmost path components when space is short. The selected terminal title
stays in the native window.

Wheel over the tab strip or the selected column's pane headers scrolls clipped
headers without scrolling the terminal; over another column it does nothing. In standalone mode these keys remain Orbit input.

### Scrolling and selection

Compatible terminal output keeps scrolling, selection, and tab/pane focus usable
between repaints. Venus retains the last presented input geometry while
refreshing content; Orbit continues parsing output and owns the anchored
history viewport. Resize, screen, workspace, and attachment changes still
require fresh presentation.

While scrolled, the clickable `↓ N rows` pill shows the last committed viewport's
wrapped display-row distance above live output (`↓ 1 row` for one). Its whole
area returns to live output through Orbit. The selected pane header reserves
space for it, preserving the directory ending when the label needs shortening.
Standalone and popup terminals use a small top-right overlay
without resizing the grid; it yields to selection, link previews, notices,
popup tab previews, and an overlapping terminal cursor. Live bottom, alternate screen, recovery, pending reflow,
and known terminal-owned scrolling hide it. Fractional preview movement never
changes the number. If the complete label cannot fit, it stays available in the
terminal's accessible description; digits are never truncated. The description
is not a live alert.

Precision touchpad movement tracks Orbit-owned retained history at twice its
native pixel distance, and a complete gesture may continue with bounded momentum
after release. A bounded Orbit-authored row window keeps multi-row movement
continuous while signed commits are in flight. Discrete wheel steps move three
retained rows without synthetic momentum. Terminal-owned mouse modes continue to
receive their canonical Orbit input instead. Hold Shift while dragging the left
mouse button to bypass that capture. Drag to select cells, double-click to
select words, triple-click to select space/tab-delimited spans including
punctuation, or click four or more times to select logical lines. Orbit's native
wide-character spacer behavior can truncate a span after a wide glyph. Hold a
drag in or above the top visible row to extend the selection upward through
history. Releasing writes Orbit's exact bounded text to both the ordinary
Wayland clipboard and primary selection; Ctrl+Shift+C remains an explicit
ordinary-clipboard copy.
Press Ctrl+Shift+V or the native Paste key to read the ordinary clipboard once.
Orbit applies normal or bracketed paste from its authoritative terminal mode.
Terminal programs can also request bounded text writes through Orbit. On Linux,
Venus sends the standard destination to the ordinary clipboard and sends the
selection or primary destination to the primary clipboard. The macOS candidate
uses Command+C and Command+V and maps every destination to the general native
pasteboard; it does not emulate a primary selection. Native proof in this
repository covers Unicode Command+C and Command+V and the empty or non-text
pasteboard notice (`VEN-C16`). Pre-consolidation M1 proof of a terminal
clipboard write and of PNG-only and oversized rejection awaits re-proof here.
A native host-access failure notice remains unaccepted.
