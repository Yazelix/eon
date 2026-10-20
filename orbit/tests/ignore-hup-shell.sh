# shellcheck shell=bash
trap "" HUP
printf '%s\n' "$$" > "$ORBIT_CHILD_PID_FILE"
while :; do sleep 1; done
