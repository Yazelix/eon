# shellcheck shell=bash
stty -echo -icanon
gate() { until IFS= read -r _ < "$ORBIT_SELECT_GATE"; do :; done; }
printf 'HELLO WORLD alpha:beta/gamma\r\n\033]0;SELECT READY\007'
gate
# Track the mouse with SGR reports, and record what a terminal-routed
# sequence sends: a left press and its release.
printf '\033[?1000h\033[?1006h\033]0;TRACKING\007'
# shellcheck disable=SC2034 # ShellCheck reads the -d delimiter as a variable
IFS= read -r -d M press
# shellcheck disable=SC2034 # ShellCheck reads the -d delimiter as a variable
IFS= read -r -d m release
printf '%q %q\n' "$press" "$release" > "$ORBIT_POINTER_FILE"
printf '\033]0;POINTER READ\007'
gate
exit 3
