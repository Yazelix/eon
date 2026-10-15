# Interaction

## Hyperlinks

Hover an explicit OSC 8 link to highlight it and preview its actual target.
Ctrl+left click opens it on Linux; Command+left click opens it on macOS.
Ctrl+Shift+C on Linux or Command+C on macOS copies the hovered target
unless the terminal has selected text, in which case it copies that selection.
Physical M1 proof of both Command actions on an exact hovered link predates this
repository and awaits re-proof here.
Ordinary clicks retain terminal mouse reporting and selection behavior;
ordinary URL-looking text is not detected as a link. The accessibility tree
exposes each current visible target as an Open link followed by a Copy button.
A changed frame retires both. Hover and actions pause during scroll animation
and renderer recovery.

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

The accepted Eon snapshot supplies the authoritative terminal endpoint: the
active tab's selected popup, otherwise its selected Orbit pane. An empty body
detaches the presentation without stopping hidden Sessions.
While the window is open, Venus re-inspects Eon every 250 ms so accepted
workspace changes from another client appear without a click or restart.
Recovery continues only while that exact attachment remains current and live;
endpoint replacement or authoritative offline state cancels obsolete retry
state.

### Workspace controls

Click a tab or pane header to select it. The fixed controls in the Eon Bar are
`+` for a tab, `?` for Shortcuts when needed, and `×` to close the exact active tab. Only
the empty region between tabs and controls drags an undecorated window.

| Keys | Action |
|---|---|
| Alt+1 … Alt+9, Alt+0 | Select tab positions 1 … 10 |
| Alt+H / Alt+L | Walk tabs |
| Alt+K / Alt+J | Walk panes |
| Ctrl+Alt+H / Ctrl+Alt+L | Move the active tab |
| Ctrl+Alt+K / Ctrl+Alt+J | Move the selected pane |
| Alt+Shift+T | Request a pending tab |
| Alt+M | Create a pane |
| Alt+Shift+W | Close the active non-final tab |
| Alt+Shift+N | Open an independent Eon window when supervised by Eon |
| Alt+Z | Open Project; Eon supplies other popup shortcuts |
| Alt+/ | Open or close Shortcuts |
| Alt+Shift+B | Show or hide the bottom shortcut hints |

Eon decides which actions are available and owns popup lifetime and directory
changes. A repeated press does not duplicate a structural action. Switching
tabs retains each tab's popup selection. F6 cycles terminal, tabs, and visible
panes. Arrow keys traverse focused tabs or panes; Tab and Shift+Tab traverse
header controls; Enter or Space activates one. Escape returns chrome focus to
the terminal. With terminal focus, Escape, Ctrl+C, Tab, and Enter reach Orbit as
ordinary input.

Shortcuts shows fixed bindings and enabled popup entries. Wheel, arrow keys,
Page Up/Down, Home, and End scroll it. Closing with Escape or Alt+/ restores the
previous focus without sending a terminal key or workspace action.
Alt+Shift+N also works while Shortcuts is open. A failed new-window request
appears as a local notice in the current window.

The bottom hints bar starts visible and reserves its own compact native row.
Alt+Shift+B returns that space to the terminal or restores the bar without
changing the selected tab, pane, popup or focus. The choice lasts for that
window, including across resize. Hints use the current bindings and enabled
popup labels, and follow native terminal, tab and pane focus. Shared modifiers
appear once per group: C means Ctrl, A means Alt, and S means Shift.
Compact groups spread across the outlined bar with equal gaps and the outer
groups near its edges; a lone group stays centered. Hint spacing stays compact.
Only modifier headers have padded rounded backgrounds. The main region and
footer share side margins and one outer frame with a straight separator,
matching lower corner radii and the main frame's bottom inset. Hiding the footer
returns the row and its bottom gap to terminal content.
Coral red modifier badges use plain aliases, such as C-A, before mint keys and gray
labels. The compact tab range is 1..9; Alt+0 still selects
the tenth tab. The move hint groups H/L for tabs and K/J for panes.
Copy/Paste and Open link stay in full Shortcuts, without footer hints.
Full Shortcuts and screen-reader labels retain complete chords and action
names. Narrow windows drop whole hints, keeping help and the visibility toggle
longest; Alt+/ always opens the complete viewer.

The header `?` disappears when the visible bar fits every applicable hint. It
stays available when the bar is hidden or incomplete, and while the button has
keyboard focus. Hints are read-only text, including for screen readers; they
add no keyboard stops. Eon has no minor modes yet.

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

Wheel over the tab strip or pane headers scrolls clipped headers without
scrolling the terminal. In standalone mode these keys remain Orbit input.

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
drag in the top visible row to extend the selection upward through history. Releasing writes
Orbit's exact bounded text to both the ordinary Wayland clipboard and primary
selection; Ctrl+Shift+C remains an explicit ordinary-clipboard copy.
Press Ctrl+Shift+V or the native Paste key to read the ordinary clipboard once.
Orbit applies normal or bracketed paste from its authoritative terminal mode.
Terminal programs can also request bounded text writes through Orbit. On Linux,
Venus sends the standard destination to the ordinary clipboard and sends the
selection or primary destination to the primary clipboard. The macOS candidate
uses Command+C and Command+V and maps every destination to the general native
pasteboard; it does not emulate a primary selection. Pre-consolidation M1
proof, awaiting re-proof here, covered one Unicode terminal clipboard write, one
Unicode Command+V paste, and visible rejection of empty, PNG-only, and
oversized values. A native host-access failure
notice remains unaccepted.
