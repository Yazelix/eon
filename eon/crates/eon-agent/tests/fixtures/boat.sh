#!/usr/bin/env bash
# Test-only CLI double: no account, network, real VM or remote code execution.
set -eu
root=$BOAT_TEST_ROOT
while [[ $1 == --* ]]; do
  case "$1" in
    --no-update|--json) shift ;;
    --org) [ "$2" = personal ]; shift 2 ;;
    *) exit 90 ;;
  esac
done
verb=$1; shift
printf '%s\n' "$verb" >> "$root/calls"
mode=$(cat "$root/mode" 2>/dev/null || true)
id=bx_test
[ ! -e "$root/deleted" ] || id=bx_next
case "$verb" in
env)
  [ "$1" = info ]; [ "$2" = eon-builds ]
  if [ "$mode" = preflight-hold ]; then
    touch "$root/preflight"
    while :; do sleep 1; done
  fi
  github=false
  [ "$mode" != unsafe ] || github=true
  printf '{"safeForThirdParties":true,"passGithub":%s,"passSecrets":false,"passSandboxCredentials":false,"passAgentsCredentials":false,"proxyAiCredentials":false,"variables":0,"secretFiles":[],"selectedRepositories":[],"latestVersionNumber":1}\n' "$github"
  ;;
limits)
  if [ "$mode" = mutate-source ]; then echo changed > "$root/../source"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-queue ]; then chmod 000 "$root/../state/slots/queue/.lock"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-slot ]; then chmod 000 "$root/../legacy/old-1.lock"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-holder ]; then chmod 500 "$root/../state/slots/holders"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-fallback-record ]; then for ticket in "$root/../state/slots/queue"/*.json; do chmod 000 "$ticket"; done; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = unreachable ]; then echo '{"ok":false}'; exit 1; fi
  used=2000
  [ "$mode" != exhausted ] || used=90000
  tier=trial
  status=trialing
  plan=trial
  active_plan=box_20
  case "$mode" in paid*) tier=paid; status=active; plan=box_20 ;; esac
  if [ "$mode" = paid-mismatch ]; then active_plan=box_100; fi
  if [ -e "$root/attached" ]; then
    [ "$mode" != paid-after-gate-mismatch ] || active_plan=box_100
    [ "$mode" != post-reserve-loss ] || used=90000
    if [ "$mode" = post-plan-loss ]; then tier=paid; status=active; plan=box_20; active_plan=box_100; fi
  fi
  printf '{"canStart":true,"activeSandboxes":0,"keepGoing":{"enabled":false},"nextStartDollars":0,"machineTypes":[{"type":"default","available":true}],"starts":{"minute":{"remaining":3},"hour":{"remaining":20},"day":{"remaining":74}},"planIncluded":{"vmHoursRemaining":555,"periodEnd":"2099-01-01T00:00:00.000Z"},"subscriptionRemainingSeconds":1998000,"accessTier":"%s","subscriptionStatus":"%s","sandboxPlanKey":"%s","activeSandboxPlanKey":"%s","trialComputeCapSeconds":90000,"trialComputeUsedSeconds":%s,"subscriptionTrialEndsAt":"2099-01-01T00:00:00.000Z"}\n' "$tier" "$status" "$plan" "$active_plan" "$used"
  ;;
new)
  if [ "$mode" = create-refused ]; then echo '{"ok":false}'; exit 1; fi
  [[ " $* " == *' --environment eon-builds '* ]]
  [[ " $* " == *' --no-env '* ]]
  echo running > "$root/state"
  echo '{"event":"creating"}'
  printf '{"sandbox":{"id":"%s"}}\n' "$id"
  echo '{"event":"ready"}'
  ;;
resume)
  [ "$1" = bx_test ]
  if [ "$mode" = resume-hold ]; then
    touch "$root/preflight"
    while :; do sleep 1; done
  fi
  [ ! -e "$root/old-command" ] || touch "$root/revived-old-command"
  [[ " $* " == *' --environment eon-builds '* ]]
  [[ " $* " == *' --no-env '* ]]
  echo running > "$root/state"
  echo '{"sandbox":{"id":"bx_test"}}'
  ;;
info)
  [ "$1" = "$id" ] || { echo '{"ok":false}'; exit 1; }
  state=$(cat "$root/state")
  if [ "$state" = stopping ]; then
    remaining=$(cat "$root/stop-polls")
    if [ "$remaining" = 0 ]; then state=stopped; echo stopped > "$root/state";
    else echo "$((remaining-1))" > "$root/stop-polls"; fi
  fi
  type=default
  [ "$mode" != wrong-type ] || type=large
  printf '{"id":"%s","state":"%s","type":"%s","holdsCreatorLogins":false,"environment":"eon-builds","environmentVersion":1}\n' "$id" "$state" "$type"
  ;;
push)
  [ "$1" = "$id" ]; [ "$3" = --to ]; [ "$4" = /home/user/eon ]
  if [ -d "$root/upload" ]; then rm -r "$root/upload"; fi
  cp -a "$2" "$root/upload"
  [ ! -e "$root/upload/.git" ]; [ ! -e "$root/upload/secret" ]
  echo 'pushed captured tree'
  ;;
exec)
  [ "$1" = "$id" ]; shift
  case "$1" in
    --detach)
      [[ " $* " == *' -- env -i HOME=/home/user USER=user LOGNAME=user PATH=/usr/local/bin:/usr/bin:/bin bash --noprofile --norc -c '* ]]
      until [ "$1" = eon-build ]; do shift; done
      shift; tree=$1; shift 3; run_id=$1
      git -C "$root/upload" init -q
      git -C "$root/upload" add -f --all
      [ "$(git -C "$root/upload" write-tree)" = "$tree" ]
      code=$(cat "$root/code" 2>/dev/null || echo 0)
      complete=true
      [ "$mode" != incomplete-log ] || complete=false
      identity_absent=true
      [ "$mode" != identity-present ] || identity_absent=false
      printf '{"run_id":"%s","tree":"%s","gate_ran":true,"source_verified":true,"build_identity_absent":%s,"log_complete":%s,"exit_code":%s}\n' "$run_id" "$tree" "$identity_absent" "$complete" "$code" > "$root/result.json"
      head -c 1100000 /dev/zero | tr '\0' x > "$root/output.log"
      echo '{"processId":123}'
      ;;
    --attach)
      touch "$root/attached"
      echo '{"type":"stdout","data":"remote stdout\n"}'
      echo '{"type":"stderr","data":"remote stderr\n"}'
      case "$mode" in post-*) sleep 35 ;; esac
      while [ -e "$root/hold" ]; do sleep 0.05; done
      if [ "$mode" = disconnected ]; then touch "$root/old-command"; exit 1; fi
      ;;
    --kill) rm -f "$root/hold"; echo '{"success":true}' ;;
    *) echo 'remote preparation' ;;
  esac
  ;;
scp)
  case "$1" in
    *:*/result.json) cp "$root/result.json" "$2" ;;
    *:*/output.log) cp "$root/output.log" "$2" ;;
    *) exit 91 ;;
  esac
  ;;
stop)
  if [ "$mode" = delayed-stop ]; then echo stopping > "$root/state"; echo 6 > "$root/stop-polls";
  else echo stopped > "$root/state"; fi
  rm -f "$root/hold"
  echo '{"success":true}'
  ;;
delete)
  [ "$1" = --yes ]; [ "$2" = "$id" ]
  touch "$root/deleted"
  rm -f "$root/old-command"
  echo deleted > "$root/state"
  echo '{"success":true}'
  ;;
usage) echo '{"seconds":1,"dollars":0.00001,"running":false}' ;;
list)
  if [ "$mode" = reconcile-unreachable ]; then exit 1; fi
  if [ "$mode" = reconcile-hold ]; then
    touch "$root/preflight"
    while :; do sleep 1; done
  fi
  echo '{"sandboxes":[]}'
  ;;
snapshots)
  named='[]'
  [ ! -f "$root/snapshot" ] || named='[{"name":"eon-warm","status":"ready","sourceSandboxId":"bx_test","snapshotId":"im_test"}]'
  used=0
  [ "$mode" != paid-full ] || used=10
  printf '{"named":%s,"namedAllowance":{"included":10,"used":%s,"extra":0}}\n' "$named" "$used"
  ;;
snapshot)
  [ "$mode" != paid-full ]
  [ "$1" = bx_test ]; [ "$2" = eon-warm ]
  touch "$root/snapshot"
  echo '{"success":true}'
  ;;
*) exit 92 ;;
esac
