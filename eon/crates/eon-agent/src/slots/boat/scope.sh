# Shared process glue; Rust selects class, cap and reclaim threshold.
unit='' aslice='' scope=()
scope_setup() {
  local cg='' s
  if command -v systemd-run >/dev/null 2>&1 && systemctl --user show -p Version >/dev/null 2>&1; then
    { read -r cg </proc/self/cgroup; } 2>/dev/null
    case $cg in
      */agent.slice/agent-*.slice/*) s=${cg#*/agent.slice/}; s=${s%%/*}; [[ $s =~ ^agent-[A-Za-z0-9_]+\.slice$ ]] && aslice=$s ;;
    esac
    unit=heavy-$$-${EPOCHREALTIME//[!0-9]/}
    # Rune's own watcher stops at a memory.peak of 7.9 GiB, which counts clean page cache: its B7
    # workspace build stopped at 8.49 GB with 3.8 GB anon and 4.5 GB inactive file (2026-11-18).
    # MemoryHigh one GiB under the cap makes the kernel reclaim that cache first; the cap, the swap
    # rule and the stop are unchanged.
    mhigh=()
    [ "${mhigh_gib:-0}" -eq 0 ] || mhigh=(-p "MemoryHigh=${mhigh_gib}G")
    scope=(systemd-run --user --scope --quiet --unit "$unit" ${aslice:+"--slice=$aslice"}
      -p "MemoryMax=${mcap}G" "${mhigh[@]}" -p MemorySwapMax=0 -p OOMPolicy=kill --)
  else
    echo "heavy.sh: no user systemd here; this run has no memory cap" >&2
  fi
  command -v choom >/dev/null 2>&1 && scope+=(choom -n 1000 --)
}

scope_end() {
  local i=0 k v as
  while :; do
    as='' mres=''
    while IFS='=' read -r k v; do
      case $k in ActiveState) as=$v ;; Result) mres=$v ;; esac
    done < <(systemctl --user show -p ActiveState -p Result "$unit.scope" 2>/dev/null)
    case $as in active|activating|deactivating) [ "$i" -lt 20 ] && { i=$((i + 1)); sleep 0.1; continue; } ;; esac
    break
  done
  [ "$as" = failed ] && systemctl --user reset-failed "$unit.scope" 2>/dev/null
}

# Keep the workload cgroup alive until the outside observer reads its counters.
boat_scope_controller() {
  local managed_scope=$1 cap=$2 high=$3 margin=$4
  shift 4
  local path parent group ancestor key effective available total ceiling code before after
  local job_oom job_kills group_kills guest_kills own global other logged complete lines
  local drained=false vmstat_readable=false remaining groups
  path=$(cat /proc/self/cgroup) || return 96
  [[ $path == 0::/*/"$managed_scope" ]] || return 96
  parent=/sys/fs/cgroup${path#0::}
  # systemd attaches a scope process to the scope root. Move ourselves first,
  # then enable the controller on the now-empty root's children.
  mkdir "$parent/controller" || return 96
  printf '%s\n' "$BASHPID" > "$parent/controller/cgroup.procs" || return 96
  [[ $(cat /proc/self/cgroup) == "$path/controller" ]] || return 96
  group=$parent/workload
  printf '+memory\n' > "$parent/cgroup.subtree_control" || return 96
  mkdir "$group" || return 96
  effective=$((12 * 1024 * 1024 * 1024))
  ancestor=$parent
  while [ "$ancestor" != /sys/fs/cgroup ]; do
    for key in memory.max memory.high; do
      read -r path < "$ancestor/$key" || return 96
      [ "$path" != max ] || continue
      [[ $path =~ ^[0-9]{1,18}$ ]] || return 96
      if (( path < effective )); then effective=$path; fi
    done
    ancestor=${ancestor%/*}
  done
  read -r total available < <(awk '
    /^MemTotal:/ { t=$2*1024 }
    /^MemAvailable:/ { a=$2*1024 }
    END { if (t>0 && a>0) printf "%.0f %.0f\n",t,a }' /proc/meminfo) || return 96
  [[ $total =~ ^[0-9]{1,18}$ && $available =~ ^[0-9]{1,18}$ ]] || return 96
  (( available <= total )) || return 96
  ceiling=$((available > margin ? available - margin : 0))
  if (( ceiling > effective )); then ceiling=$effective; fi
  if (( cap > ceiling )); then
    printf '{"available":%s,"total":%s,"ceiling":%s}\n' "$available" "$total" "$ceiling" > /home/user/eon-run/headroom-failed
    return 96
  fi
  printf '%s\n' "$cap" > "$group/memory.max" || return 96
  printf '%s\n' "${high:-max}" > "$group/memory.high" || return 96
  printf '0\n' > "$group/memory.swap.max" || return 96
  printf '1\n' > "$group/memory.oom.group" || return 96
  [[ $(cat "$group/memory.max") == "$cap" &&
     $(cat "$group/memory.high") == "${high:-max}" &&
     $(cat "$group/memory.swap.max") == 0 &&
     $(cat "$group/memory.oom.group") == 1 &&
     -w "$group/cgroup.kill" ]] || return 96

  before=$(awk '$1=="oom_kill" {print $2}' /proc/vmstat)
  [[ $before =~ ^[0-9]+$ ]] || return 96
  dmesg --raw > /home/user/eon-run/kernel-before || return 96
  [ -s /home/user/eon-run/kernel-before ] || return 96
  command -v bwrap >/dev/null || return 96
  local resolver=()
  case $(readlink -f /etc/resolv.conf) in
    /run/*) path=$(readlink -f /etc/resolv.conf); resolver=(--ro-bind "$path" "$path") ;;
  esac

  code=0
  (
    printf '%s\n' "$BASHPID" > "$group/cgroup.procs" || exit 96
    printf '1000\n' > /proc/self/oom_score_adj || exit 96
    path=$(cat /proc/self/cgroup) || exit 96
    [[ $path == "0::${group#/sys/fs/cgroup}" && $(cat /proc/self/oom_score_adj) == 1000 ]] || exit 96
    # Only the observer can see the guest manager or write cgroup controls.
    # Preserve network access for Nix's local-store fetches.
    exec setpriv --no-new-privs --inh-caps=-all --ambient-caps=-all \
      bwrap --unshare-user --unshare-pid --unshare-ipc --die-with-parent --new-session \
      --cap-drop ALL --ro-bind / / --proc /proc --dev /dev \
      --tmpfs /run "${resolver[@]}" --ro-bind /sys/fs/cgroup /sys/fs/cgroup \
      --bind /home/user /home/user --bind /nix /nix \
      --ro-bind /home/user/eon-run /home/user/eon-run \
      --bind /home/user/eon-run/workload /home/user/eon-run/workload \
      --bind /home/user/eon-run/workload/tmp /tmp \
      --unsetenv XDG_RUNTIME_DIR --unsetenv DBUS_SESSION_BUS_ADDRESS \
      --unsetenv DBUS_SYSTEM_BUS_ADDRESS \
      -- bash --noprofile --norc -c 'boat_workload "$@"' eon-workload "$@" \
      5>/home/user/eon-run/preparation-started
  ) || code=$?

  # SIGKILL delivery to the main child can precede the last descendant's exit.
  # Wait briefly before sampling the final counters and log.
  for ((remaining=0; remaining<40; remaining++)); do
    if awk '$1=="populated" && $2==0 {found=1} END {exit !found}' "$group/cgroup.events"; then
      drained=true
      break
    fi
    sleep .05
  done
  read -r job_oom job_kills group_kills < <(awk '
    $1=="oom" {o=$2} $1=="oom_kill" {k=$2} $1=="oom_group_kill" {g=$2}
    END {if (o!="" && k!="") print o,k,(g=="" ? "unknown" : g)}' "$group/memory.events") || return 96
  [[ $job_oom =~ ^[0-9]+$ && $job_kills =~ ^[0-9]+$ ]] || return 96
  after=$(awk '$1=="oom_kill" {print $2}' /proc/vmstat)
  complete=false
  guest_kills=0 own=0 global=0 other=0
  if [[ $after =~ ^[0-9]+$ ]] && (( after >= before )); then
    vmstat_readable=true
    guest_kills=$((after - before))
    if dmesg --raw > /home/user/eon-run/kernel-after; then
      # A wrapped or altered baseline cannot exclude a VM-wide event.
      lines=$(wc -l < /home/user/eon-run/kernel-before)
      if head -n "$lines" /home/user/eon-run/kernel-after | cmp -s /home/user/eon-run/kernel-before -; then
        read -r own global other logged groups < <(
          tail -n "+$((lines + 1))" /home/user/eon-run/kernel-after |
            awk -v cg="${group#/sys/fs/cgroup}" '
              /oom-kill:constraint=/ {
                group=0
                if (awaiting) other++
                awaiting=0
                if (index($0,"constraint=CONSTRAINT_NONE,")) global++
                else if (index($0,"constraint=CONSTRAINT_MEMCG,") &&
                         index($0,",oom_memcg=" cg ",") &&
                         match($0,/,pid=[0-9]+,/)) {
                  victim=substr($0,RSTART+5,RLENGTH-6)
                  if (victim in primary) other++
                  primary[victim]=1
                } else other++
              }
              /[Oo]ut of memory.*: Killed process [0-9]+ / {
                logged++
                if (!/Memory cgroup out of memory: Killed process /) {
                  global++
                  group=0
                } else if (match($0,/Killed process [0-9]+ /)) {
                  victim=substr($0,RSTART+15,RLENGTH-16)
                  if (victim in primary) {
                    own++
                    awaiting=1
                    delete primary[victim]
                  } else if (!group || awaiting) other++
                }
              }
              /Tasks in .* are going to be killed due to memory[.]oom[.]group set$/ {
                group=awaiting && index($0,"Tasks in " cg " are going to be killed")
                if (group) groups++
                else other++
                awaiting=0
              }
              END {
                for (victim in primary) other++
                if (awaiting) other++
                print own+0,global+0,other+0,logged+0,groups+0
              }')
        # Every guest kill must be logged. The kill message carries the domain
        # even when Linux rate-limits the constraint record. For a group kill
        # require one matching primary context per cgroup OOM invocation.
        if (( logged == guest_kills )) &&
           { (( guest_kills == 0 )) || [[ $group_kills == "$groups" && $groups == "$own" && $own != 0 ]]; }; then
          complete=true
        fi
      fi
    fi
  fi
  printf '{"max_bytes":%s,"high_bytes":%s,"swap_bytes":0,"available":%s,"total":%s,"ceiling":%s,"scope_oom":%s,"scope_kills":%s,"guest_kills":%s,"own_events":%s,"global_events":%s,"other_events":%s,"complete":%s,"vmstat_readable":%s,"drained":%s}\n' \
    "$cap" "${high:-null}" "$available" "$total" "$ceiling" "$job_oom" "$job_kills" "$guest_kills" "$own" "$global" "$other" "$complete" "$vmstat_readable" "$drained" > /home/user/eon-run/memory.json || return 96
  if [ "$drained" = true ]; then
    rmdir "$group" || echo 'phase=scope-cleanup-deferred' >&2
  else
    printf '1\n' > "$group/cgroup.kill" || return 96
    return 96
  fi
  return "$code"
}
