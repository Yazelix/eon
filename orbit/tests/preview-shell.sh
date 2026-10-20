stty -echo
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
for ((line = 1; line <= 100; line++)); do
  printf 'row %03d\n' "$line"
done
printf '%0200d\n' 0
printf '\033]0;PREVIEW READY\007'
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
printf '\033[?2026h\033]0;PREVIEW HELD\007'
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
printf '\033[?1000h'
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
printf '\033[?2026l'
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
printf '\033]0;PREVIEW ROUTED\007'
until IFS= read -r _ < "$ORBIT_PREVIEW_GATE"; do :; done
exit 7
