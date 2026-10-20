stty -echo -icanon
gate() { until IFS= read -r _ < "$ORBIT_TICK_GATE"; do :; done; }
# Sixty lines leave 37 rows of history above a 24-row viewport.
for i in $(seq 1 60); do printf 'line %02d\r\n' "$i"; done
printf '\033]0;TICK READY\007'
gate
# Open a synchronized update; the answer to a cursor query proves the server
# has read it before the check goes on.
printf '\033[?2026h\033[6n'
IFS= read -r -s -d R _
: > "$ORBIT_HELD_FILE"
gate
printf '\033[?2026l'
gate
exit 6
