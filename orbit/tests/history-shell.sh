stty -echo
until IFS= read -r _ < "$ORBIT_HISTORY_GATE"; do :; done
for ((line = 1; line <= 20000; line++)); do
  printf 'history %05d\n' "$line"
done
printf '\033]0;HISTORY DONE\007'
until IFS= read -r _ < "$ORBIT_HISTORY_GATE"; do :; done
exit 7
