# Raw input: the terminal driver hands this child every byte the terminal
# sends back, with no signal, carriage-return or flow-control handling, so a
# reply cannot hide a control byte. Output keeps its newline mapping. Bash
# reads and records bytes, not characters.
export LC_ALL=C
stty raw -echo opost
# One line of output puts the cursor on row 2 before the first query.
printf 'TERMINAL REPLIES\n'
cpr=$'\e\\[[0-9]+;[0-9]+R$'
da=$'\e\\[\\?[0-9;]*c$'
# Records under NAME every byte the terminal sends back up to the end of an
# answer matching END, or with no END everything within half a second. An
# answer that never ends is recorded as TIMEOUT with what did arrive. Bash
# cannot hold NUL, so it is recorded as <NUL>.
answer() {
  local reply= character wait=0.5
  [[ -n $2 ]] && wait=5
  while IFS= read -r -s -d '' -n 1 -t "$wait" character; do
    reply+=${character:-<NUL>}
    [[ -n $2 && $reply =~ $2 ]] && break
  done
  if [[ -z $2 || $reply =~ $2 ]]; then
    printf '%s %q\n' "$1" "$reply" >> "$ORBIT_REPLIES"
  else
    printf '%s TIMEOUT %q\n' "$1" "$reply" >> "$ORBIT_REPLIES"
  fi
}
# Asks the terminal QUERY and records the answer up to END.
query() {
  printf '%s' "$2"
  answer "$1" "$3"
}
# Writes BYTES, then asks for device attributes, and records every byte that
# comes back through that answer: replies arrive in stream order, so anything
# BYTES made the terminal send precedes it.
ask() {
  query "$1" "$2"$'\e[c' "$da"
}
# Uppercase hexadecimal of each byte, as XTGETTCAP names keys.
hex() {
  local text=$1 out= i
  for ((i = 0; i < ${#text}; i++)); do
    printf -v out '%s%02X' "$out" "'${text:i:1}"
  done
  printf '%s' "$out"
}
marker=PWNED-260mh15
# Shell syntax that would run the marker if it reached a shell's input,
# with both quotes and a backslash.
shell=";echo $marker;\$(echo $marker)\`echo $marker\`|sh '\"\\ #"
# Bytes a shell or line editor acts on: CR, LF, ^C, ^U and DEL, and C1
# controls raw and encoded as UTF-8. ESC, which ends any string, comes
# separately with the end of a bracketed paste.
controls=$'\r\n\x03\x15\x7f\x9b\x9c\x9d\xc2\x9b\xc2\x9c\xc2\x9d'
paste_end=$'\e[201~'
# Every program-triggered reply family, each with hostile text where the
# request carries any. Each case ends in a device attributes answer.
corpus() {
  ask "$1-baseline" ''
  # Titles and icon labels, BEL- and ST-terminated, holding the shell syntax
  # and every control byte, then each request that could report a title.
  ask "$1-title" $'\e]2;'"$shell"$'\a\e]1;'"$shell$marker"$'\e\\\e]0;'"$marker$controls$paste_end$shell"$'\a\e]2;'"$marker"$'\x03\x15\x7f\xc2\x9b\xc2\x9d'"$shell"$'\e\\\e[21t\e[20t\e[22;0t\e[23;0t\e[21t'
  # DECRQSS: a supported request, an unsupported one (DECSCL), one holding
  # the shell syntax and control bytes, and one longer than any it knows.
  ask "$1-decrqss" $'\eP$qm\e\\\eP$q"p\e\\\eP$q'"$shell$controls$paste_end"$'m\e\\\eP$q'"$(printf 'm%.0s' {1..300})"$'\e\\'
  # XTGETTCAP: a known key, the terminfo name Orbit never sets, the shell
  # syntax as a hexadecimal key, and the shell syntax unencoded.
  ask "$1-tcap" $'\eP+q436F\e\\\eP+q544E\e\\\eP+q'"$(hex "$shell")"$'\e\\\eP+q'"$shell"$'\e\\'
  # Colours: a set and query round trip, specifications holding the shell
  # syntax, then the queries; the defaults return before the next case.
  ask "$1-colors" $'\e]10;rgb:12/34/56\a\e]10;?\a\e]11;'"$shell"$';rgb:00/00/00\a\e]4;1;'"$shell$marker"$'\a\e]4;1;?\e\\\e]11;?\a\e]12;?\a\e]21;foreground=?;'"$marker"$'=?;background='"$shell"$'\e\\\e]21;background=?\a\e]110\a\e]111\a\e]112\a\e]104\a'
  # Fixed and numeric reports, and requests Orbit leaves unanswered: secondary
  # and tertiary device attributes, version,
  # answerback, colour scheme, screen checksum, unknown mode, keyboard flags,
  # and a working directory holding the shell syntax.
  ask "$1-status" $'\e[>c\e[=c\e[>q\x05\e[?996n\e[1;1;1;1;1;1*y\e[5n\e[?31337$p\e[?u\e]7;file://'"$marker/$shell"$'\a'
  # Clipboard reads, Orbit's denied OSC 52 read and Kitty reads and writes
  # whose ids hold the shell syntax, control bytes, and too many characters.
  id="x\$(echo\${IFS}$marker)|sh&&x'\"\\$controls"
  ask "$1-clipboard" $'\e]52;c;?\a\e]5522;type=read:id='"$id"$';dGV4dC9wbGFpbg==\e\\\e]5522;type=read:id='"$(printf 'A%.0s' {1..600})"$';dGV4dC9wbGFpbg==\a\e]5522;type=write:id='"$id"$'\e\\\e]5522;type=wdata:id='"$id"$'\e\\'
  # Kitty graphics queries, valid and with the shell syntax as data or id,
  # and a Glyph Protocol query, which libghostty leaves off.
  ask "$1-graphics" $'\e_Gi=31,a=q,s=1,v=1,f=24;AAAA\e\\\e_Gi=32,a=q,s=1,v=1,f=24;'"$shell"$'\e\\\e_Gi='"$marker"$',a=q;AAAA\e\\\e_25a1;s\e\\'
  # Drag and drop: a support query, one whose client id is the marker, and a
  # data request with no drop.
  ask "$1-dnd" $'\e]72;t=q\e\\\e]72;t=q:i='"$marker"$'\e\\\e]72;t=r:x=1\e\\'
  # Nothing else arrives afterwards.
  answer "$1-stray"
  # Hostile text may have been printed; the cursor returns to row 2.
  printf '\e[2;1H'
}
# Before any client attaches.
query detached $'\e[6n' "$cpr"
# Program status reports, which are never answered, then the probe, which
# gets only its fixed answer (ORB-C15). The first report is valid: its id
# and app are names, and its title and message decode to printable UTF-8
# holding the shell syntax. The second, whose message holds control bytes,
# is rejected whole.
text=$(printf '%s' "$marker$shell" | base64 -w 0)
controlled=$(printf '%s' "$marker$controls$paste_end$shell" | base64 -w 0)
ask probe $'\e]7501;state=blocked:kind=question:id='"$marker"$':app='"$marker"$':title='"$text"$':msg='"$text"$'\e\\\e]7501;state=blocked:kind=question:id='"$marker"$':msg='"$controlled"$'\a\e]7501;?\e\\'
corpus detached
until IFS= read -r _ < "$ORBIT_REPLY_GATE"; do :; done
# With a client attached.
query position $'\e[6n' "$cpr"
query attributes $'\e[c' "$da"
corpus attached
printf '\e]0;REPLIED\007'
until IFS= read -r _ < "$ORBIT_REPLY_GATE"; do :; done
exit 6
