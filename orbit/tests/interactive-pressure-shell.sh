printf '%s\n' "$$" > "$ORBIT_PID_FILE"
tick=0
while test ! -e "$ORBIT_STOP"; do
  tick=$((tick + 1))
  printf '\033]0;TICK %d\007line %d\n' "$tick" "$tick"
  sleep 0.01
done
exit 9
