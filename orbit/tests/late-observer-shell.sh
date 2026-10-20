printf '%s\n' "$$" > "$ORBIT_PID_FILE"
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;LATE ONE\007\n'
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;LATE TWO\007\n'
IFS= read -r -n 1 input
test "$input" = x || exit 42
exit 7
