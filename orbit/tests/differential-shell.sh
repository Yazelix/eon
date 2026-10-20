stty -echo
gate() { until IFS= read -r _ < "$ORBIT_DIFF_GATE"; do :; done; }
gate
# A clipboard write, first so that it reaches the first read, ahead of every
# frame of the step; then rich output: attributes, colors, a hyperlink, wide
# and combining characters, a working directory and history.
printf '\033]52;c;Y29waWVk\007'
for ((line = 1; line <= 120; line++)); do
  printf 'row %03d\n' "$line"
done
# Keep the rich cells visible in the final frame retained for this step.
printf '\033[1;31mbold red\033[0m \033[4:3mcurly\033[0m \033[38;2;1;2;3;48;5;200mdirect\033[0m '
printf '\033]8;;https://example.com/\033\\link\033]8;;\033\\ \xe5\xae\xbd e\xcc\x81\r\n'
printf '\033]7;file://localhost/tmp/differential\007'
printf '\033]0;DIFF OUTPUT\007'
# A paste, then a pressed key; a released key writes nothing.
IFS= read -r -n 5 pasted
printf 'pasted %s\r\n\033]0;DIFF PASTE %s\007' "$pasted" "$pasted"
IFS= read -r -n 1 key
printf 'key %s\r\n\033]0;DIFF KEY %s\007' "$key" "$key"
gate
printf 'size %s\r\n\033]0;DIFF SIZE\007' "$(stty size)"
gate
printf '\033[H\033[2JHELLO WORLD alpha:beta/gamma\r\n\033]0;DIFF SELECT\007'
gate
# Output while a finished host selection is presented.
printf 'kept\r\n\033]0;DIFF KEPT\007'
gate
printf 'bye\r\n\033]0;DIFF BYE\007'
exit 5
