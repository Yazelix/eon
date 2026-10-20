stty -echo -icanon
gate() { until IFS= read -r _ < "$ORBIT_CLIPBOARD_GATE"; do :; done; }
gate
# No interactive client is attached, so this write reaches nobody.
printf '\033]52;c;ZGV0YWNoZWQ=\007\033]0;DETACHED WROTE\007'
gate
# A clear, which is refused, then a standard and a primary write before the
# title of the same read.
printf '\033]52;c;\007\033]52;c;Y29waWVk\007\033]52;p;cHJpbWFyeQ==\007\033]0;COPIED\007'
gate
# A synchronized update holds the frame but not the write.
printf '\033[?2026h\033]52;s;aGVsZA==\007\033]0;HELD\007'
gate
printf '\033[?2026l'
gate
# More writes in one read than the old Orbit's bound of 64 end the attachment.
writes=''
for ((i = 0; i < 300; i++)); do writes+='\033]52;c;eA==\007'; done
printf "$writes"
gate
printf '\033]0;AFTER OVERFLOW\007'
gate
exit 5
