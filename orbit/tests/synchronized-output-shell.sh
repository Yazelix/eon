stty -echo
until IFS= read -r _ < "$ORBIT_SYNC_GATE"; do :; done
printf '\033[?2026h\033]0;SYNC PARTIAL\007partial'
until IFS= read -r _ < "$ORBIT_SYNC_GATE"; do :; done
printf '\033]0;SYNC COMPLETE\007\033[?2026l'
until IFS= read -r _ < "$ORBIT_SYNC_GATE"; do :; done
printf '\033[?2026h\033]0;SYNC WATCHDOG\007'
until IFS= read -r _ < "$ORBIT_SYNC_GATE"; do :; done
printf '\033[?2026h\033]0;SYNC EXIT\007'
exit 7
