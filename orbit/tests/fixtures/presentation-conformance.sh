#!/bin/sh

ready=$1
enrich=$2
release=$3
flooding=$4
finished=$5
stop=$6
primary=$7

printf '%s' "$primary"
: > "$ready"
while [ ! -e "$enrich" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

printf '\033[?1049h\033[2J\033[H'
printf '\033]2;rich\033\\\033]7;file:///tmp/orbit\033\\'
printf '\033]10;rgb:01/02/03\033\\\033]11;rgb:04/05/06\033\\'
printf '\033]12;rgb:07/08/09\033\\\033]4;17;rgb:0a/0b/0c\033\\'
printf '\033]8;;https://example.test\033\\'
printf '\033[1"q\033[1;2;3;4:3;5;7;8;9;53;38;2;12;34;56;48;5;17;58;2;7;8;9mA'
printf '\033[0m\033[0"q\033]8;;\033\\'
printf 'e\314\201\347\225\214'
printf '\033[4;1H\033[48;2;5;6;7m\033[2K\033[0m\033[3;5H\033[?25l\033[4 q'

while [ ! -e "$release" ] && [ ! -e "$stop" ]; do
    sleep 0.01
done
[ -e "$stop" ] && exit

running=yes
trap 'running=' INT
dd if=/dev/zero bs=65536 count=4 2>/dev/null
: > "$flooding"
while [ -n "$running" ] && [ ! -e "$stop" ]; do
    printf '%s' "$primary"
done
[ -e "$stop" ] && exit
trap - INT

printf '\033[?1049lX'
printf '\033[3'
sleep 0.02
printf '2mS\033[0m'
printf '\303'
sleep 0.02
printf '\251'
printf '\033_Ga=q;'
sleep 0.02
printf '\033\\'
printf '\033]2;final\033\\'
: > "$finished"

while [ ! -e "$stop" ]; do
    sleep 0.01
done
