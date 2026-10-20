printf '%s\n' "$$" > "$ORBIT_PID_FILE"
until IFS= read -r _ < "$ORBIT_OUTPUT_GATE"; do :; done
# Enough distinct frames to fill the stopped client's socket before Exited.
for ((line = 0; line < 100000; line++)); do printf 'STOPPED %06d\n' "$line"; done
: > "$ORBIT_EXIT_MARK"
exit 7
