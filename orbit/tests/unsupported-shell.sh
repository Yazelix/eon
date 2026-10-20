printf '%s\n' "$$" > "$ORBIT_PID_FILE"
stty -echo
IFS= read -r -n 1 first
printf '\033]0;FIRST %s\007' "$first"
IFS= read -r -n 1 second
printf '\033]0;SECOND %s\007' "$second"
exit 7
