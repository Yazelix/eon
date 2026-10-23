printf '%s\n' "$$" > "$ORBIT_PID_FILE"
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;LATE ONE\007\n'
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]0;LATE TWO\007\n'
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
title=$(printf '%1023s' '')
printf '\033]0;%s\303\251\007\n' "${title// /a}"
until IFS= read -r _ < "$ORBIT_TITLE_GATE"; do :; done
printf '\033]7;file://localhost/tmp/\377\007\033]0;LATE THREE\007\n'
IFS= read -r -n 1 input
test "$input" = x || exit 42
exit 7
