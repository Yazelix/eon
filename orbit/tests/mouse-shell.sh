stty -echo -icanon
gate() {
  until IFS= read -r _ < "$ORBIT_MOUSE_GATE"; do :; done
}
# Reports in the title, as hex, every byte read before the next "a".
report() {
  IFS= read -r -d a data
  printf '\033]0;%s %s\007' "$1" "$(printf %s "$data" | od -An -tx1 | tr -d ' \n')"
}
gate
for ((line = 1; line <= 200; line++)); do
  printf 'row %03d\n' "$line"
done
printf '\033]0;MOUSE FREE\007'
gate
printf '\033[?2026h\033]0;MOUSE HELD\007'
gate
printf '\033[?2026l'
gate
printf '\033[?1000h\033[?1006h\033]0;MOUSE TRACKED\007'
report TRACKED
gate
printf '\033[?1000l\033[?1006l\033[?1049h\033[?1007h\033]0;MOUSE ALTERNATE\007'
report ALTERNATE
gate
exit 5
