# shellcheck shell=bash
# The shell ignores SIGHUP and runs, with job control, a foreground job in its
# own process group that ignores SIGHUP too: only a signal to the terminal's
# foreground group reaches it.
trap "" HUP
set -m
printf '%s\n' "$$" > "$ORBIT_CHILD_PID_FILE"
bash -c 'trap "" HUP; printf "%s\n" "$$" > "$ORBIT_JOB_PID_FILE"; while :; do sleep 1; done'
