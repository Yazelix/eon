# shellcheck shell=bash
stty -echo
until IFS= read -r _ < "$ORBIT_KEY_GATE"; do :; done
for ((line = 1; line <= 200; line++)); do
  printf 'row %03d\n' "$line"
done
# Bracketed paste is on before the client can paste.
printf '\033[?2004h\033]0;KEY READY\007'
# The bracketed paste arrives first, then the pressed key; the released key
# sends nothing.
IFS= read -r -N 13 pasted
IFS= read -r -n 1 key
if [[ $pasted == $'\e[200~x\e[201~' ]]; then kind=B; else kind=U; fi
printf '\033]0;READ %s %s\007' "$kind" "$key"
until IFS= read -r _ < "$ORBIT_KEY_GATE"; do :; done
exit 3
