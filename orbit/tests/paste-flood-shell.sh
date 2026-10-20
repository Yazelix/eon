# Without imaxbel, macOS's terminal rings no bell when the input queue fills.
stty -echo -imaxbel
printf '\033]0;PASTE READY\007'
gate() {
  until IFS= read -r _ < "$ORBIT_PASTE_GATE"; do :; done
}
# The child reads no input until the first gate opens.
gate
# Then it prints every line it reads, so its output competes with its input.
awk '
  { print; lines++ }
  $0 == "MARK" { printf "\033]0;MARK %d\007", lines; fflush() }
  $0 == "END" { printf "\033]0;LINES %d\007", lines; fflush(); exit }
'
gate
exit 5
