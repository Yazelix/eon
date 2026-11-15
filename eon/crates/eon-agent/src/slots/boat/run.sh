# Only fixed process glue; Rust validates argv, source identity, policy and budget.
set -eu
tree=$1
relative=$2
jobs=$3
run_id=$4
gate_warm=$5
dependency_expression=$6
shift 6
while [ ! -f /home/user/eon-run/go ]; do sleep 1; done
umask 077
exec 3>&1 4>&2
exec > >(tee -a /home/user/eon-run/output.log >&3 3>&- 4>&-)
stdout_tee=$!
exec 2> >(tee -a /home/user/eon-run/output.log >&4 3>&- 4>&-)
stderr_tee=$!
gate_ran=false
source_verified=false
build_identity_absent=false
finish() {
  code=$?
  trap - EXIT
  [ ! -f /home/user/eon-run/gate-started ] || gate_ran=true
  [ ! -f /home/user/eon-run/build-identity-absent ] || build_identity_absent=true
  echo "phase=complete exit_code=$code"
  exec >&3 2>&4 3>&- 4>&-
  log_complete=true
  wait "$stdout_tee" || log_complete=false
  wait "$stderr_tee" || log_complete=false
  printf '{"run_id":"%s","tree":"%s","source_verified":%s,"gate_ran":%s,"build_identity_absent":%s,"log_complete":%s,"exit_code":%s}\n' \
    "$run_id" "$tree" "$source_verified" "$gate_ran" "$build_identity_absent" "$log_complete" "$code" > /home/user/eon-run/result.tmp
  mv /home/user/eon-run/result.tmp /home/user/eon-run/result.json
  exit 0
}
trap finish EXIT
if [ "${ASCII_TOKEN+x}" ] || [ "${BOAT_ID+x}" ]; then
  echo 'build-provider-identity: refused before preparation' >&2
  exit 97
fi
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0
cd /home/user/eon
git init -q
git -c core.hooksPath=/dev/null add -f --all
[ "$(git write-tree)" = "$tree" ] || { echo 'source tree mismatch'; exit 98; }
source_verified=true
GIT_AUTHOR_DATE=2000-01-01T00:00:00Z GIT_COMMITTER_DATE=2000-01-01T00:00:00Z \
  git -c core.hooksPath=/dev/null -c user.name='Eon build' -c user.email=build@invalid commit -qm 'Captured build tree'
echo "verified-tree=$tree"
echo "source-commit=$(git rev-parse HEAD)"
export PATH=/home/user/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$PATH
if ! command -v nix >/dev/null; then
  echo 'phase=bootstrap nix=2.34.7'
  curl --fail --silent --show-error --location --max-time 300 \
    https://releases.nixos.org/nix/nix-2.34.7/nix-2.34.7-x86_64-linux.tar.xz \
    -o /home/user/eon-run/nix.tar.xz
  echo 'eafe5042404e818505e28c5ca3d0885f3ec45c31f955489a25bb38258f87560e  /home/user/eon-run/nix.tar.xz' | sha256sum -c -
  mkdir /home/user/eon-run/nix-dist
  tar -xJf /home/user/eon-run/nix.tar.xz --strip-components=1 -C /home/user/eon-run/nix-dist
  sudo mkdir -p /nix
  sudo chown user /nix
  /home/user/eon-run/nix-dist/install --no-daemon --yes --no-channel-add --no-modify-profile
fi
mkdir -p /home/user/.config/nix
cat > /home/user/.config/nix/nix.conf <<EOF
experimental-features = nix-command flakes
extra-substituters = https://eon-yazelix.cachix.org
extra-trusted-public-keys = eon-yazelix.cachix.org-1:kPIp7u0Gjoz0GNzmRqeaqs798vU2raSOUpK+3xi7oK0=
max-jobs = 1
cores = $jobs
accept-flake-config = false
EOF
export CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR=/home/user/eon-target
echo 'phase=inputs'
# Evaluation fetches only inputs consumed by this build, not optional recording apps.
echo 'phase=warmup-plan'
if [ "$gate_warm" = 1 ]; then
nix build --dry-run .#checks.x86_64-linux.eon-tests .#default
# Package inputs can include the check itself; warm only the check's prerequisites.
nix derivation show .#checks.x86_64-linux.eon-tests > /home/user/eon-run/derivations.json
nix eval --raw --impure --expr "$dependency_expression" \
  --apply 'f: f { doc = builtins.fromJSON (builtins.readFile /home/user/eon-run/derivations.json); }' \
  > /home/user/eon-run/dependencies
echo 'phase=warmup'
mapfile -t dependencies < /home/user/eon-run/dependencies
if [ "${#dependencies[@]}" -gt 0 ]; then nix build -L --no-link "${dependencies[@]}"; fi
else
  nix develop -L -c bash --noprofile --norc -c \
    '[ ! "${ASCII_TOKEN+x}" ] && [ ! "${BOAT_ID+x}" ]'
fi
cd "/home/user/eon/$relative"
# Boat supplies ambient capabilities; unprivileged Bubblewrap rejects them.
setpriv --inh-caps=-all --ambient-caps=-all bash --noprofile --norc -c '
  while read -r key bits; do
    case "$key" in
      CapInh:|CapPrm:|CapEff:|CapAmb:) [ -z "${bits//0/}" ] || exit 97 ;;
    esac
  done < /proc/self/status
  if [ "${ASCII_TOKEN+x}" ] || [ "${BOAT_ID+x}" ]; then
    echo build-provider-identity=present >&2
    exit 97
  fi
  touch /home/user/eon-run/build-identity-absent
  echo build-provider-identity=absent
  echo execution-capabilities=none
  touch /home/user/eon-run/gate-started
  echo phase=gate
  exec "$@"
' eon-build "$@" 3>&- 4>&-
