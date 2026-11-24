# shellcheck shell=bash
stty -echo -icanon
gate() { until IFS= read -r _ < "$ORBIT_TICK_GATE"; do :; done; }
# Sixty lines leave 37 rows of history above a 24-row viewport.
for i in $(seq 1 60); do printf 'line %02d\r\n' "$i"; done
printf '\033]0;TICK READY\007'
gate
# Open a synchronized update; the answer to a cursor query proves the server
# has read it before the check goes on.
hold() {
    local marker=$1
    printf '\033[?2026h\033[6n'
    # shellcheck disable=SC2034 # ShellCheck reads the -d delimiter as a variable
    IFS= read -r -s -d R _
    : > "$marker"
    gate
}
hold "$ORBIT_HELD_FILE"
printf '\033[?2026l'
gate
printf '\033[?1000h\033[?1006h\033]0;TICK TRACKING\007'
# shellcheck disable=SC2034 # ShellCheck reads the -d delimiter as a variable
IFS= read -r -d M press
# shellcheck disable=SC2034 # ShellCheck reads the -d delimiter as a variable
IFS= read -r -d m release
printf '%sM%sm' "$press" "$release" > "$ORBIT_HELD_FILE.mouse"
printf '\033]0;TICK MOUSE READ\007'
gate
hold "$ORBIT_HELD_FILE.down"
printf 'live end\033[?2026l'
gate
printf '\033[?1049h\033]0;TICK ALTERNATE\007'
gate
exit 6
