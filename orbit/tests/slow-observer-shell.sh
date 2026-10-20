until IFS= read -r _ < "$ORBIT_SLOW_GATE"; do :; done
padding=$(printf '%01000d' 0)
for ((n = 1; n <= 1000; n++)); do
  printf '\033]0;%04d%s\007' "$n" "$padding"
done
printf '\033]0;SLOW LAST\007'
until IFS= read -r _ < "$ORBIT_SLOW_GATE"; do :; done
printf 'output under the same title\n'
until IFS= read -r _ < "$ORBIT_SLOW_GATE"; do :; done
exit 7
