printf '%s\n' "$$" > "$ORBIT_PID_FILE"
stty -echo
until IFS= read -r _ < "$ORBIT_READY_GATE"; do :; done
until IFS= read -r _ < "$ORBIT_RECONNECT_GATE"; do :; done
printf '\033]0;RECONNECT UPDATED\007'
: > "$ORBIT_OUTPUT_FILE"
IFS= read -r -n 1 input
test "$input" = x || exit 42
printf '\033]0;FIRST INPUT\007'
IFS= read -r -n 1 second
test "$second" = y || exit 43
printf '\nINPUT:%s%s\n' "$input" "$second"
exit 7
