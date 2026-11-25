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
id=bx_test
active=0
if [ -d "$root/pool" ]; then
  case "$verb" in
    new)
      [ "$1" = --name ]
      id="bx_${2#eon-build-}"
      root="$root/pool/$id"
      mkdir "$root"
      printf '%s\n' "$2" > "$root/name"
      printf '%s\n' "${BOAT_TEST_VM_MODE:-}" > "$root/mode"
      [ "${BOAT_TEST_VM_MODE:-}" != hold ] || touch "$root/hold"
      if [ "${BOAT_TEST_VM_MODE:-}" = publish-fails ]; then
        chmod 500 "$root/../../../state/slots/boat"
      fi
      ;;
    limits)
      for machine in "$root/pool/"*/state; do
        [ -f "$machine" ] || continue
        case "$(cat "$machine")" in running|stopping) active=$((active + 1)) ;; esac
      done
      ;;
    list)
      printf '{"sandboxes":['
      comma=
      for vm in "$root/pool/"*; do
        [ -f "$vm/state" ] || continue
        state=$(cat "$vm/state")
        [ "$state" != deleted ] || continue
        printf '%s{"id":"%s","name":"%s","state":"%s","type":"%s"}' \
          "$comma" "${vm##*/}" "$(cat "$vm/name")" "$state" "$(cat "$vm/type")"
        comma=,
      done
      printf ']}\n'
      exit 0
      ;;
    info|exec|push|stop|usage|delete|scp)
      case "$verb" in
        delete) id=$2 ;;
        scp) id=${1%%:*} ;;
        *) id=$1 ;;
      esac
      root="$root/pool/$id"
      [ -d "$root" ] || exit 1
      ;;
  esac
else
  [ ! -e "$root/deleted" ] || id=bx_next
fi
mode=$(cat "$root/mode" 2>/dev/null || true)
case "$verb" in
env)
  [ "$1" = info ]; [ "$2" = eon-builds ]
  if [ "$mode" = preflight-hold ]; then
    touch "$root/preflight"
    while :; do sleep 1; done
  fi
  github=false
  [ "$mode" != unsafe ] || github=true
  version=2
  [ "$mode" != environment-version ] || version=1
  printf '{"safeForThirdParties":true,"passGithub":%s,"passSecrets":false,"passSandboxCredentials":false,"passAgentsCredentials":false,"proxyAiCredentials":false,"variables":0,"secretFiles":[],"selectedRepositories":[],"latestVersionNumber":%s}\n' "$github" "$version"
  ;;
limits)
  if [ -f "$root/policy-probed" ]; then
  if [ "$mode" = mutate-source ]; then echo changed > "$root/../source"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-queue ]; then chmod 000 "$root/../state/slots/queue/.lock"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-slot ]; then chmod 000 "$root/../legacy/old-1.lock"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-local-holder ]; then chmod 500 "$root/../state/slots/holders"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = break-fallback-record ]; then chmod 500 "$root/../state/slots/queue"; echo '{"ok":false}'; exit 1; fi
  if [ "$mode" = unreachable ]; then echo '{"ok":false}'; exit 1; fi
  fi
  touch "$root/policy-probed"
  large=true
  [ "$mode" != large-unavailable ] || large=false
  used=2000
  if [ -f "$root/trial-used" ]; then used=$(cat "$root/trial-used"); fi
  [ "$mode" != exhausted ] || used=90000
  tier=trial
  status=trialing
  plan=trial
  active_plan=box_20
  case "$mode" in paid*) tier=paid; status=active; plan=box_20 ;; standard*) tier=standard; status=active; plan=box_20 ;; esac
  if [ "$mode" = paid-mismatch ]; then active_plan=box_100; fi
  hours=555 seconds=1998000 pack=0 cap=90000
  [ "$mode" != paid ] || { hours=1; seconds=3600; pack=30000; cap=null; used=null; }
  if [ -e "$root/attached" ]; then
    case "$mode" in post-reserve-loss*) used=90000 ;; esac
    if [ "$mode" = post-plan-loss ]; then tier=paid; status=active; plan=box_20; active_plan=box_100; fi
  fi
  printf '{"canStart":true,"activeSandboxes":%s,"keepGoing":{"enabled":false},"nextStartDollars":0,"machineTypes":[{"type":"default","available":true},{"type":"large","available":%s}],"starts":{"minute":{"remaining":3},"hour":{"remaining":20},"day":{"remaining":74}},"planIncluded":{"vmHoursRemaining":%s,"periodEnd":"2099-01-01T00:00:00.000Z"},"subscriptionRemainingSeconds":%s,"packBalanceSeconds":%s,"accessTier":"%s","subscriptionStatus":"%s","sandboxPlanKey":"%s","activeSandboxPlanKey":"%s","trialComputeCapSeconds":%s,"trialComputeUsedSeconds":%s,"subscriptionTrialEndsAt":"2099-01-01T00:00:00.000Z"}\n' "$active" "$large" "$hours" "$seconds" "$pack" "$tier" "$status" "$plan" "$active_plan" "$cap" "$used"
  ;;
new)
  if [ "$mode" = create-refused ]; then echo '{"ok":false}'; exit 1; fi
  [[ " $* " == *' --environment eon-builds '* ]]
  [[ " $* " == *' --no-env '* ]]
  # Only the pinned image, with captures off, may be started.
  [[ " $* " == *' --from eon-builds-nix-2-34-7-large-klt8h-v1 --no-snapshots '* ]]
  printf '%s\n' "$*" > "$root/new-args"
  rm -f "$root/marker-verified"
  for ((i=1;i<$#;i++)); do
    if [ "${!i}" = --type ]; then j=$((i+1)); printf '%s\n' "${!j}" > "$root/type"; break; fi
  done
  echo running > "$root/state"
  echo '{"event":"creating"}'
  printf '{"sandbox":{"id":"%s"}}\n' "${BOAT_TEST_COLLISION:-$id}"
  echo '{"event":"ready"}'
  ;;
info)
  [ "$1" = "$id" ] || { echo '{"ok":false}'; exit 1; }
  if [ "$mode" = info-hold ] && [ ! -e "$root/held" ]; then
    touch "$root/held" "$root/preflight"
    while :; do sleep 1; done
  fi
  state=$(cat "$root/state")
  if [ "$state" = stopping ]; then
    remaining=$(cat "$root/stop-polls")
    if [ "$remaining" = 0 ]; then state=stopped; echo stopped > "$root/state";
    else echo "$((remaining-1))" > "$root/stop-polls"; fi
  fi
  type=$(cat "$root/type" 2>/dev/null || echo default)
  [ "$mode" != wrong-type ] || type=small
  # vm-inactive: the first report after the start shows a state work cannot run in.
  if [ "$mode" = vm-inactive ] && [ ! -e "$root/info-read" ]; then state=error; fi
  touch "$root/info-read"
  version=',"environmentVersion":2'
  [ "$mode" != identity-missing ] || version=
  printf '{"id":"%s","state":"%s","type":"%s","holdsCreatorLogins":false,"environment":"eon-builds"%s}\n' "$id" "$state" "$type" "$version"
  ;;
push)
  [ "$1" = "$id" ]; [ "$3" = --to ]; [ "$4" = /home/user/eon ]
  # Source reaches only a VM whose image marker was verified.
  [ -e "$root/marker-verified" ]
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
      shift; printf '%s\0' "$1" "${@:7}" > "$root/detached"
      printf '%s\n' "$3 ${4:-null} $5" > "$root/requested-memory"
      read -r cap high margin < <(cat "$root/observed-memory" 2>/dev/null || echo '8589934592 null 2147483648')
      git -C "$root/upload" init -q
      git -C "$root/upload" add -f --all
      [ "$(git -C "$root/upload" write-tree)" = "$tree" ]
      code=$(cat "$root/code" 2>/dev/null || echo 0)
      complete=true
      [ "$mode" != incomplete-log ] || complete=false
      identity_absent=true
      [ "$mode" != identity-present ] || identity_absent=false
      prepared=true gate=true source=true headroom=null
      oom=0 kills=0 guest=0 own=0 global=0 other=0
      case "$mode" in
        job-oom|prep-oom|mixed-oom) code=137; oom=1; kills=1; guest=1; own=1 ;;
        vm-oom|unknown-oom) code=137; kills=1; guest=1 ;;
      esac
      case "$mode" in
        prep-oom) gate=false; source=false ;;
        vm-oom) global=1 ;;
        mixed-oom) global=1; guest=2 ;;
      esac
      memory=$(printf '{"max_bytes":%s,"high_bytes":%s,"swap_bytes":0,"available":%s,"total":17179869184,"ceiling":%s,"scope_oom":%s,"scope_kills":%s,"guest_kills":%s,"own_events":%s,"global_events":%s,"other_events":%s,"complete":true,"vmstat_readable":true,"drained":true}' "$cap" "$high" "$((cap+margin))" "$cap" "$oom" "$kills" "$guest" "$own" "$global" "$other")
      case "$mode" in
        headroom|memory-unavailable) prepared=false gate=false source=false code=96 memory=null ;;
      esac
      if [ "$mode" = headroom ]; then headroom='{"available":1073741824,"total":8589934592,"ceiling":0}'; fi
      printf '{"run_id":"%s","tree":"%s","gate_ran":%s,"source_verified":%s,"build_identity_absent":%s,"log_complete":%s,"exit_code":%s,"preparation_started":%s,"memory":%s,"headroom_failed":%s}\n' "$run_id" "$tree" "$gate" "$source" "$identity_absent" "$complete" "$code" "$prepared" "$memory" "$headroom" > "$root/result.json"
      head -c 1100000 /dev/zero | tr '\0' x > "$root/output.log"
      echo '{"processId":123}'
      ;;
    --attach)
      touch "$root/attached"
      n=$(( $(cat "$root/attach-count" 2>/dev/null || echo 0) + 1 ))
      echo "$n" > "$root/attach-count"
      # Data frames hold literal \n escapes, as Boat's JSON does.
      frame() { printf '{"type":"%s","data":"%s"}\n' "$1" "$2"; }
      finish() { touch "$root/finished"; echo '{"type":"exit","status":"exited","exitCode":0}'; exit 0; }
      case "$mode" in
        attach-drop|attach-drop-exited|attach-big-frame)
          if [ "$n" = 1 ]; then
            frame stdout 'line1\nline2\n'
            frame stderr 'err1\n'
            if [ "$mode" = attach-big-frame ]; then
              printf '{"event":"error","error":"%s"}\n' "$(head -c 1048576 /dev/zero | tr '\0' e)"
              printf 'boat: first\nsecond \033[2J\n' >&2
            else
              echo '{"event":"error","error":"stream reset","code":"stream_reset"}'
              echo 'boat: reading the output failed' >&2
            fi
            [ "$mode" != attach-drop-exited ] || touch "$root/finished"
            exit 1
          fi
          # The replay arrives in other chunk sizes, then the rest of the output.
          frame stdout 'li'; frame stdout 'ne1\nline2\n'; frame stderr 'err1\n'
          frame stdout 'line3\n'
          finish ;;
        attach-zero-early|attach-error-zero)
          frame stdout 'line1\n'
          if [ "$n" = 1 ]; then
            if [ "$mode" = attach-error-zero ]; then
              echo '{"event":"error","error":"stream lost"}'
              echo '{"type":"exit","status":"exited","exitCode":0}'
            fi
            exit 0
          fi
          frame stdout 'line3\n'
          finish ;;
        attach-quiet|status-outage)
          frame stdout 'line1\n'
          limit=6; [ "$mode" != status-outage ] || limit=3
          [ "$n" -ge "$limit" ] || exit 1
          frame stdout 'line3\n'
          finish ;;
        status-blip)
          # One status read fails at the drop; the next attach then holds past patience.
          frame stdout 'line1\n'
          [ "$n" -ge 2 ] || exit 1
          for _ in $(seq 15); do frame stdout '.'; sleep 2; done
          frame stdout 'line3\n'
          finish ;;
        status-slow-running)
          # The second status read starts before patience and answers running after it.
          frame stdout 'line1\n'
          [ "$n" -ge 3 ] || exit 1
          frame stdout 'line3\n'
          finish ;;
        attach-trickle)
          for i in $(seq 1 "$n"); do frame stdout "tick$i\\n"; done
          [ "$n" -ge 5 ] || exit 1
          finish ;;
        post-reserve-loss-drops)
          frame stdout "tick$n\\n"; sleep 0.5; exit 1 ;;
      esac
      frame stdout 'remote stdout\n'
      frame stderr 'remote stderr\n'
      # Waits print a frame every 2 s: the test host's 20 s patience watches silence.
      case "$mode" in post-*) for _ in $(seq 18); do frame stdout '.'; sleep 2; done ;; esac
      ticks=0
      while [ -e "$root/hold" ]; do
        sleep 0.05; ticks=$((ticks + 1))
        if [ "$ticks" -ge 40 ]; then ticks=0; frame stdout '.'; fi
      done
      if [ "$mode" = disconnected ]; then touch "$root/old-command"; exit 1; fi
      finish
      ;;
    --status)
      printf 'status\n' >> "$root/status-calls"
      if [ "$mode" = disconnected ]; then echo '{"ok":false}'; exit 1; fi
      calls=$(wc -l < "$root/status-calls")
      fail=
      case "$mode" in
        status-outage) [ "$calls" -gt 2 ] || fail=1 ;;
        status-blip|status-slow-running) [ "$calls" != 1 ] || fail=1 ;;
      esac
      if [ -n "$fail" ]; then echo '{"event":"error","error":"api unavailable"}'; exit 1; fi
      [ "$mode" != status-slow-running ] || [ "$calls" != 2 ] || sleep 22
      if [ -e "$root/finished" ]; then
        echo '{"type":"command.status","running":false,"exitCode":0}'
      else
        echo '{"type":"command.status","running":true,"exitCode":null}'
      fi
      ;;
    --kill)
      printf 'kill\n' >> "$root/kills"
      [ ! -e "$root/slow-kill" ] || sleep 40
      rm -f "$root/hold"; echo '{"success":true}' ;;
    --)
      if [[ " $* " == *' -- env -i PATH=/usr/bin:/bin LC_ALL=C bash --noprofile --norc -c '*' eon-image 7a5da4f0f0a1bf9de41b47695ae48f94ed115b7145cba5b941bcee2019dd708f ' ]]; then
        case "$mode" in
          marker-absent) exit 94 ;;
          marker-mismatch) exit 95 ;;
        esac
        touch "$root/marker-verified"
      else
        # The credential probe and setup run only after the marker check.
        [ -e "$root/marker-verified" ]
        echo 'remote preparation'
      fi
      ;;
    *) echo 'remote preparation' ;;
  esac
  ;;
scp)
  case "$1" in
    # The result exists once the command has finished, as run.sh writes it last.
    *:*/result.json) [ -e "$root/finished" ] || exit 1; cp "$root/result.json" "$2" ;;
    *:*/output.log) [ ! -e "$root/log-copy-fails" ] || exit 1; cp "$root/output.log" "$2" ;;
    *) exit 91 ;;
  esac
  ;;
stop)
  if [ "$mode" = publish-fails ]; then
    touch "$root/../../retiring-unpublished"
    while [ ! -e "$root/../../continue-retirement" ]; do sleep 0.05; done
  fi
  if [ "$mode" = delayed-stop ]; then echo stopping > "$root/state"; echo 6 > "$root/stop-polls";
  elif [ "$mode" = stuck-stop ] || [ -e "$root/slow-retire" ]; then echo stopping > "$root/state"; echo 1000 > "$root/stop-polls";
  else echo stopped > "$root/state"; fi
  rm -f "$root/hold"
  echo '{"success":true}'
  ;;
delete)
  [ "$1" = --yes ]; [ "$2" = "$id" ]
  [ ! -e "$root/delete-refused" ] || exit 1
  [ ! -e "$root/slow-retire" ] || sleep 10
  touch "$root/deleted"
  rm -f "$root/old-command"
  echo deleted > "$root/state"
  echo '{"success":true}'
  ;;
usage)
  [ ! -e "$root/usage-fails" ] || exit 1
  [ ! -e "$root/slow-retire" ] || sleep 10
  echo '{"seconds":1,"dollars":0.00001,"running":false}' ;;
list)
  if [ "$mode" = reconcile-unreachable ] || [ -e "$root/list-fails" ]; then exit 1; fi
  # The deleted run's VM still listed: its deletion stays unconfirmed.
  if [ -e "$root/list-keeps" ]; then echo '{"sandboxes":[{"id":"bx_test"}]}'; exit 0; fi
  if [ "$mode" = reconcile-hold ]; then
    touch "$root/preflight"
    while :; do sleep 1; done
  fi
  echo '{"sandboxes":[]}'
  ;;
snapshots)
  # The pinned image's named row; modes change it before or after the start.
  id_field=9c882067-a76c-4580-9a53-c90ad574441e
  [ "$mode" != image-wrong-id ] || id_field=9c882067-0000-4000-8000-000000000000
  if [ "$mode" = image-drift ] && [ -e "$root/new-args" ]; then id_field=other; fi
  named=$(printf '[{"name":"eon-warm","status":"ready","sourceSandboxId":"bx_old","snapshotId":"im_old"},{"name":"eon-builds-nix-2-34-7-large-klt8h-v1","status":"ready","sourceSandboxId":"bx_vfxzz24a","snapshotId":"%s","type":"large"}]' "$id_field")
  [ "$mode" != image-absent ] || named='[{"name":"eon-warm","status":"ready","sourceSandboxId":"bx_old","snapshotId":"im_old"}]'
  printf '{"named":%s,"namedAllowance":{"included":10,"used":2,"extra":0}}\n' "$named"
  ;;
*) exit 92 ;;
esac
