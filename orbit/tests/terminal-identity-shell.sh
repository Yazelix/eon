printf '\033]0;TERM=%s COLORTERM=%s TERMINFO_DIRS=%s\007' "$TERM" "$COLORTERM" "$TERMINFO_DIRS"
printf 'BEFORE CLEAR %s\n' 1 2 3
until IFS= read -r _ < "$ORBIT_IDENTITY_GATE"; do :; done
clear
status=$?
printf 'CLEARED %s' "$status"
printf '\033]0;CLEARED\007'
until IFS= read -r _ < "$ORBIT_IDENTITY_GATE"; do :; done
exit 0
