stty -echo
until IFS= read -r _ < "$ORBIT_KEY_GATE"; do :; done
for ((line = 1; line <= 200; line++)); do
  printf 'row %03d\n' "$line"
done
printf '\033]0;KEY READY\007'
# The paste arrives first, then the pressed key; the released key sends nothing.
IFS= read -r -n 1 first
IFS= read -r -n 1 second
printf '\033]0;READ %s%s\007' "$first" "$second"
until IFS= read -r _ < "$ORBIT_KEY_GATE"; do :; done
exit 3
