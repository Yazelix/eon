stty -echo -imaxbel
printf '\033]0;FLUSH READY\007'
until IFS= read -r _ < "$ORBIT_FLUSH_GATE"; do :; done
# Output the client does not read yet, while its input waits unread.
for ((i = 0; i < 3000; i++)); do
  printf "line %04d %070d\r\n" "$i" 0
done
printf '\033]0;FLOODED\007'
until IFS= read -r _ < "$ORBIT_FLUSH_GATE"; do :; done
exit 4
