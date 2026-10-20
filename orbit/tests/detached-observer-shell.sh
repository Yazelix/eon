printf '%s\n' "$$" > "$ORBIT_PID_FILE"
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;DETACHED ONE\007\n'
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;DETACHED TWO\007\n'
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
# Opens a synchronized update and stays quiet, so only the watchdog ends it.
printf '\033[?2026h\033]0;DETACHED HELD\007'
IFS= read -r -n 1 input
test "$input" = x || exit 42
exit 7
