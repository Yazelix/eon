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
# A rich alternate screen: styles, a hyperlink, wide, combining and emoji
# graphemes, and a moved cursor.
printf '\033[?2026l\033[?1049h\033[H\033]8;;https://example.test\033\\\033[1;38;2;12;34;56;48;5;17mRICH 界 e\xcc\x81 \xf0\x9f\x91\x8d\xf0\x9f\x8f\xbd \xf0\x9f\x87\xaf\xf0\x9f\x87\xb5 \xf0\x9f\x91\xa8\xe2\x80\x8d\xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x91\xa7\033[0m\033]8;;\033\\\033[3;5H\033]0;ALTERNATE\007'
gate
# A full reset keeps emoji clustering on.
printf '\033c\xf0\x9f\x91\x8d\xf0\x9f\x8f\xbd \xf0\x9f\x87\xaf\xf0\x9f\x87\xb5 \xf0\x9f\x91\xa8\xe2\x80\x8d\xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x91\xa7\033]0;RESET\007'
gate
exit 4
