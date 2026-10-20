stty -echo
gate() { until IFS= read -r _ < "$ORBIT_GATE"; do :; done; }
gate
# A synchronized update held before any client attaches.
printf '\033[?2026hHELD'
: > "$ORBIT_MARK.held"
gate
printf '\033]0;RELEASED\007\033[?2026l'
gate
# One that is never released, so the watchdog ends it.
printf '\r\n\033[?2026hWATCHDOG\033]0;WATCHDOG\007'
: > "$ORBIT_MARK.watchdog"
gate
# A rich alternate screen: styles, a hyperlink, wide and combining graphemes,
# and a moved cursor.
printf '\033[?2026l\033[?1049h\033[H\033]8;;https://example.test\033\\\033[1;38;2;12;34;56;48;5;17mRICH 界 e\xcc\x81\033[0m\033]8;;\033\\\033[3;5H\033]0;ALTERNATE\007'
gate
exit 4
