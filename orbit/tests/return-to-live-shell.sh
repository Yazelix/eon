stty -echo
gate() {
  until IFS= read -r _ < "$ORBIT_LIVE_GATE"; do :; done
}
gate
for ((line = 1; line <= 1500; line++)); do
  printf 'row %04d\n' "$line"
done
printf '\033]0;LIVE READY\007'
gate
printf '\033[?2026h\033]0;LIVE HELD\007'
gate
printf '\033[?2026l'
gate
printf '\033[?1000h\033]0;LIVE ROUTED\007'
gate
printf '\033[?1000l\033[?1007l\033[?1049h\033]0;LIVE ALTERNATE\007'
gate
printf '\033[?1049l\033]0;LIVE PRIMARY\007'
# No return to live sends PTY input, so the first byte the child reads is the paste.
IFS= read -r -n 1 input
test "$input" = x || exit 42
: > "$ORBIT_LIVE_PASTED"
gate
exit 7
