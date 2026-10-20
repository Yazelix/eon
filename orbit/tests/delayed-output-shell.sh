stty -echo
printf '\033[?1004h'
until IFS= read -r _ < "$ORBIT_DELAY_GATE"; do :; done
printf 'L\033]0;DELAYED\007'
IFS= read -r -n 3 focus
test "$focus" = $'\033[I' || exit 45
IFS= read -r -n 1 input
test "$input" = x || exit 42
printf '\033]0;FIRST INPUT\007\n'
IFS= read -r -n 1 second
test "$second" = y || exit 43
IFS= read -r -n 1 third
test "$third" = z || exit 44
printf '\033]0;FINAL INPUT\007'
printf '\nINPUT:%s%s%s\n' "$input" "$second" "$third"
exit 7
