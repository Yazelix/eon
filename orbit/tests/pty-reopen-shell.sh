until IFS= read -r _ < "$ORBIT_REOPEN_GATE"; do :; done
# Close every descriptor on the terminal, so the PTY master reads EIO while
# this child lives, for longer than the host's two-second reap bound.
exec 0<&- 1>&- 2>&-
sleep 2.5
exec 0</dev/tty 1>/dev/tty 2>&1
printf '\033]0;BACK\007'
until IFS= read -r _ < "$ORBIT_REOPEN_GATE"; do :; done
exit 4
