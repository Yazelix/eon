# shellcheck shell=bash
printf '\033]0;TERM=%s COLORTERM=%s TERMINFO_DIRS=%s\007' "$TERM" "$COLORTERM" "$TERMINFO_DIRS"
printf 'BEFORE CLEAR %s\n' 1 2 3
until IFS= read -r _ < "$ORBIT_IDENTITY_GATE"; do :; done
clear
status=$?
printf 'CLEARED %s' "$status"
printf '\033]0;CLEARED\007'
until IFS= read -r _ < "$ORBIT_IDENTITY_GATE"; do :; done
if [ "$(uname -s)" = Linux ]; then
    printf '\r\033[2K\033[38;2;210;220;230m'
    tput Smulx 3 2>/dev/null
    smulx_status=$?
    tput Setulc 1193046 2>/dev/null
    setulc_status=$?
    printf 'COLOURED\033[0m\r\nCAPABILITIES %s %s' "$smulx_status" "$setulc_status"
    printf '\033]0;UNDERLINED\007'
    until IFS= read -r _ < "$ORBIT_IDENTITY_GATE"; do :; done
fi
exit 0
