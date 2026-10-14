#!/bin/sh

begin=$1
parsed=$2
payload=$3
emitted=$4
end=$5
timeout=$6
timeout_held=$7
stop=$8

stty raw -echo
while [ ! -e "$begin" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

printf '\033[?2026h\033[6n'
dd bs=1 count=6 of=/dev/null 2>/dev/null
: > "$parsed"
while [ ! -e "$payload" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

printf '\033]52;c;c3luY2VkIGNvcHk=\033\\'
dd if=/dev/zero bs=16384 count=1 2>/dev/null | tr '\000' X
printf '\033]2;sync-final\033\\'
: > "$emitted"
while [ ! -e "$end" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

printf '\033[?2026l'
while [ ! -e "$timeout" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

printf '\033[?2026h\033]2;sync-timeout\033\\\033[H\033[6n'
dd bs=1 count=6 of=/dev/null 2>/dev/null
: > "$timeout_held"
while [ ! -e "$stop" ]; do
    sleep 0.01
done
