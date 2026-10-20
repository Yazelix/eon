# shellcheck shell=bash
IFS= read -r -n 1 input
test "$input" = x
title=$(printf '%01000d' 0)
for n in $(seq 1 1000); do
  printf '\033]0;%03d%s\007' "$n" "$title"
done
exit 7
