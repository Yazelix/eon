# shellcheck shell=bash
# Pinned image marker on the guest: 0 verified, 94 absent or unreadable, 95 different.
# $1 is the expected recipe digest. Ownership and mode are read before content, so a
# marker the build user cannot read is still judged by its mode.
marker=/opt/eon-boat/recipe.sha256
if [ -L "$marker" ]; then exit 95; fi
[ -e "$marker" ] || exit 94
[ -f "$marker" ] || exit 95
identity=$(stat -c '%u:%g:%a' -- "$marker") || exit 94
[ "$identity" = 0:0:444 ] || exit 95
content=$(cat -- "$marker" && printf x) || exit 94
[ "$content" = "$1"$'\n'x ] || exit 95
exit 0
