stty -echo
until IFS= read -r _ < "$ORBIT_SCROLL_GATE"; do :; done
for ((line = 1; line <= 200; line++)); do
  printf 'row %03d\n' "$line"
done
printf '\033]0;SCROLL READY\007'
until IFS= read -r _ < "$ORBIT_SCROLL_GATE"; do :; done
printf '\033[?2026h\033]0;SCROLL HELD\007'
until IFS= read -r _ < "$ORBIT_SCROLL_GATE"; do :; done
printf '\033[?2026l'
until IFS= read -r _ < "$ORBIT_SCROLL_GATE"; do :; done
printf '\033[?1000h\033]0;SCROLL ROUTED\007'
until IFS= read -r _ < "$ORBIT_SCROLL_GATE"; do :; done
printf '\033[?1000l\033]0;SCROLL FREE\007'
# No scroll sends PTY input, so the first byte the child reads is the paste.
IFS= read -r -n 1 input
test "$input" = x || exit 42
exit 7
