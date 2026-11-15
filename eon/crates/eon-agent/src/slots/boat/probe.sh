set -euo pipefail
LC_ALL=C
credential_pattern='(^|_)(TOKEN|API_KEY|SECRET)$|^SSH_AUTH_SOCK$'
refused=false
# The pinned noninteractive Bash lacks compgen; fixed prefixes expand names only.
credential_names=$(
  for prefix in {A..Z} {a..z} _; do
    eval 'printf "%s\n" "${!'"$prefix"'@}"' || exit 97
  done
) || exit 97
while IFS= read -r name; do
  case "$name" in
    ASCII_TOKEN|BOAT_ID)
      printf 'credential-probe: sandbox identity name allowed: %s\n' "$name"
      continue
      ;;
  esac
  if [[ ${name^^} =~ $credential_pattern ]]; then
    if [ -z "${!name-}" ]; then
      printf 'credential-probe: empty credential placeholder: %s\n' "$name"
      continue
    fi
    printf 'credential-probe: credential variable present: %s\n' "$name" >&2
    refused=true
  fi
done <<< "$credential_names"
[ "$refused" = false ] || exit 97
metadata=/home/user/.config/ascii/boat/config.json
if [ -e "$metadata" ] || [ -L "$metadata" ]; then
  if ! python3 - <<'PY'
import json
import os
import stat
import sys

def unique_object(pairs):
    result = dict(pairs)
    if len(result) != len(pairs):
        raise ValueError()
    return result

try:
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    with os.fdopen(os.open('/home/user/.config/ascii/boat/config.json', flags)) as file:
        if not stat.S_ISREG(os.fstat(file.fileno()).st_mode):
            raise ValueError()
        metadata = json.load(file, object_pairs_hook=unique_object)
    if (not isinstance(metadata, dict)
            or set(metadata) != {'api_url', 'channel'}
            or not all(isinstance(value, str) for value in metadata.values())):
        raise ValueError()
except Exception:
    sys.exit(97)
PY
  then
    echo 'credential-probe: Boat metadata schema refused' >&2
    exit 97
  fi
  echo 'credential-probe: neutral Boat metadata schema verified'
fi
for file in .codex/auth.json .claude/.credentials.json .config/gh/hosts.yml .git-credentials .netrc .aws/credentials .config/gcloud/application_default_credentials.json; do
  if [ -e "/home/user/$file" ]; then
    printf 'credential-probe: credential file present: %s\n' "$file" >&2
    exit 97
  fi
done
for file in /home/user/.ssh/id_*; do
  if [ -f "$file" ]; then
    echo 'credential-probe: SSH private identity present' >&2
    exit 97
  fi
done
if git config --global --get-all credential.helper >/dev/null 2>&1; then
  echo 'credential-probe: Git credential helper present' >&2
  exit 97
fi
echo 'credential-probe: no nonempty caller credential variables, login files or Git helper'
