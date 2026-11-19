# shellcheck shell=bash
until IFS= read -r _ < "$ORBIT_EXIT_GATE"; do :; done
exit 7
