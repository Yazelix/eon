printf '%s\n' "$$" > "$ORBIT_PID_FILE"
until IFS= read -r _ < "$ORBIT_STALE_GATE"; do :; done
exit 7
