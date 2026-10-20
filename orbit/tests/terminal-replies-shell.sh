stty -echo -icanon
# One line of output puts the cursor on row 2 before the first query.
printf 'TERMINAL REPLIES\n'
# Asks the terminal one question and records its answer, or TIMEOUT.
query() {
  printf '%s' "$2"
  if IFS= read -r -s -t 5 -d "$3" reply; then
    printf '%s %q\n' "$1" "$reply" >> "$ORBIT_REPLIES"
  else
    printf '%s TIMEOUT\n' "$1" >> "$ORBIT_REPLIES"
  fi
}
# Before any client attaches.
query detached $'\033[6n' R
until IFS= read -r _ < "$ORBIT_REPLY_GATE"; do :; done
# With a client attached.
query position $'\033[6n' R
query attributes $'\033[c' c
printf '\033]0;REPLIED\007'
until IFS= read -r _ < "$ORBIT_REPLY_GATE"; do :; done
exit 6
