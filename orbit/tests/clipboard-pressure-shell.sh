stty -echo -icanon
gate() { until IFS= read -r _ < "$ORBIT_CLIPBOARD_GATE"; do :; done; }
# 524,286 bytes of text: under the 1 MiB bound of one read's writes.
payload=$(yes eHh4 | head -n 174762 | tr -d '\n')
gate
# One write per gate, each followed by a title the observer waits for, until
# the writes no reader takes fill the client's output queue.
for ((i = 1; i <= 40; i++)); do
  printf '\033]52;c;%s\007\033]0;W%d\007' "$payload" "$i"
  gate
done
printf '\033]0;AFTER PRESSURE\007'
gate
exit 6
