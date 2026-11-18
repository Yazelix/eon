# Only fixed process glue; Rust validates argv, source identity, policy and budget.
set -eu
tree=$1
relative=$2
jobs=$3
run_id=$4
gate_warm=$5
dependency_expression=$6
cap=$7
high=$8
margin=$9
shift 9
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
  [ ! -f /home/user/eon-run/workload/source-verified ] || source_verified=true
  preparation_started=false
  [ ! -s /home/user/eon-run/preparation-started ] || preparation_started=true
  memory=null headroom=null
  [ ! -f /home/user/eon-run/memory.json ] || memory=$(cat /home/user/eon-run/memory.json)
  [ ! -f /home/user/eon-run/headroom-failed ] || headroom=$(cat /home/user/eon-run/headroom-failed)
  [ ! -f /home/user/eon-run/workload/gate-started ] || gate_ran=true
  [ ! -f /home/user/eon-run/workload/build-identity-absent ] || build_identity_absent=true
  echo "phase=complete exit_code=$code"
  exec >&3 2>&4 3>&- 4>&-
  log_complete=true
  wait "$stdout_tee" || log_complete=false
  wait "$stderr_tee" || log_complete=false
  printf '{"run_id":"%s","tree":"%s","source_verified":%s,"gate_ran":%s,"build_identity_absent":%s,"log_complete":%s,"exit_code":%s,"preparation_started":%s,"memory":%s,"headroom_failed":%s}\n' \
    "$run_id" "$tree" "$source_verified" "$gate_ran" "$build_identity_absent" "$log_complete" "$code" "$preparation_started" "$memory" "$headroom" > /home/user/eon-run/result.tmp
  mv /home/user/eon-run/result.tmp /home/user/eon-run/result.json
  exit 0
}
trap finish EXIT
if [ "${ASCII_TOKEN+x}" ] || [ "${BOAT_ID+x}" ]; then
  echo 'build-provider-identity: refused before preparation' >&2
  exit 97
fi
export PATH=/home/user/.nix-profile/bin:/nix/var/nix/profiles/default/bin:$PATH
manager_runtime=/run/user/$(id -u)
[ -S "$manager_runtime/bus" ] || exit 96
# The guest image supplies a single-user local store. Refuse daemon or remote
# stores before preparation, and bind the client to local for its descendants.
[ ! -e /nix/var/nix/daemon-socket/socket ] || exit 96
[ -w /nix/store ] || exit 96
export NIX_REMOTE=local
scratch=/home/user/eon-run/workload/tmp
mkdir -p /home/user/.config/nix "$scratch" || exit 96
scratch_fs=$(stat -f -c %T "$scratch") || exit 96
case "$scratch_fs" in
  tmpfs|ramfs) echo 'phase=memory-scope-refused scratch is memory-backed' >&2; exit 96 ;;
esac
cat > /home/user/.config/nix/nix.conf <<EOF
experimental-features = nix-command flakes
extra-substituters = https://eon-yazelix.cachix.org
extra-trusted-public-keys = eon-yazelix.cachix.org-1:kPIp7u0Gjoz0GNzmRqeaqs798vU2raSOUpK+3xi7oK0=
max-jobs = 1
cores = $jobs
accept-flake-config = false
store = local
builders =
EOF
export NIX_USER_CONF_FILES=/home/user/.config/nix/nix.conf
store=$(nix config show store) || exit 96
builders=$(nix config show builders) || exit 96
[ "$store" = local ] && [ -z "$builders" ] || exit 96
boat_workload() {
  set -eu
  local tree=$1 relative=$2 jobs=$3 gate_warm=$4 dependency_expression=$5
  shift 5
  # Acknowledge sandbox entry and close the observer's fd before any build code.
  printf 'started\n' >&5 || exit 96
  exec 5>&-
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0
cd /home/user/eon
git init -q
git -c core.hooksPath=/dev/null add -f --all
[ "$(git write-tree)" = "$tree" ] || { echo 'source tree mismatch'; exit 98; }
touch /home/user/eon-run/workload/source-verified
GIT_AUTHOR_DATE=2000-01-01T00:00:00Z GIT_COMMITTER_DATE=2000-01-01T00:00:00Z \
  git -c core.hooksPath=/dev/null -c user.name='Eon build' -c user.email=build@invalid commit -qm 'Captured build tree'
echo "verified-tree=$tree"
echo "source-commit=$(git rev-parse HEAD)"
export CARGO_BUILD_JOBS="$jobs" CARGO_TARGET_DIR=/home/user/eon-target
echo 'phase=inputs'
# Evaluation fetches only inputs consumed by this build, not optional recording apps.
echo 'phase=warmup-plan'
if [ "$gate_warm" = 1 ]; then
nix build --dry-run .#checks.x86_64-linux.eon-tests .#default
# Package inputs can include the check itself; warm only the check's prerequisites.
nix derivation show .#checks.x86_64-linux.eon-tests > /home/user/eon-run/workload/derivations.json
nix eval --raw --impure --expr "$dependency_expression" \
  --apply 'f: f { doc = builtins.fromJSON (builtins.readFile /home/user/eon-run/workload/derivations.json); }' \
  > /home/user/eon-run/workload/dependencies
echo 'phase=warmup'
mapfile -t dependencies < /home/user/eon-run/workload/dependencies
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
  touch /home/user/eon-run/workload/build-identity-absent
  echo build-provider-identity=absent
  echo execution-capabilities=none
  touch /home/user/eon-run/workload/gate-started
  echo phase=gate
  exec "$@"
' eon-build "$@" 3>&- 4>&-
}
export -f boat_workload boat_scope_controller
code=0
XDG_RUNTIME_DIR="$manager_runtime" DBUS_SESSION_BUS_ADDRESS="unix:path=$manager_runtime/bus" \
systemd-run --user --scope --quiet --unit "eon-boat-$run_id" \
  -p Delegate=memory -p OOMPolicy=continue -- \
  bash --noprofile --norc -c 'boat_scope_controller "$@"' eon-controller \
  "eon-boat-$run_id.scope" "$cap" "$high" "$margin" "$tree" "$relative" "$jobs" "$gate_warm" "$dependency_expression" "$@" 3>&- 4>&- || code=$?
if [ ! -s /home/user/eon-run/preparation-started ]; then
  echo "phase=memory-scope-refused launch_exit=$code" >&2
  exit 96
fi
exit "$code"
