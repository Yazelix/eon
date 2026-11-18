# Test-only kernel and process boundary for the real guest controller.
set -eu
group=$FIXTURE_CGROUP$FIXTURE_SCOPE/workload
proc=$FIXTURE_PROC
case $0 in
  */systemd-run)
    [ -S "$XDG_RUNTIME_DIR/bus" ]
    [ "$DBUS_SESSION_BUS_ADDRESS" = "unix:path=$XDG_RUNTIME_DIR/bus" ]
    [ "${FIXTURE_LAUNCH:-ok}" != fail ] || exit 1
    while [ "$1" != -- ]; do shift; done
    shift
    exec "$@"
    ;;
  */nix)
    case "$*" in
      "config show store")
        [ -f "$NIX_USER_CONF_FILES" ]
        [ "${FIXTURE_NIX_STORE:-local}" != fail ] || exit 1
        echo "${FIXTURE_NIX_STORE:-local}"
        ;;
      "config show builders")
        [ "${FIXTURE_NIX_BUILDERS:-}" != fail ] || exit 1
        printf '%s\n' "${FIXTURE_NIX_BUILDERS:-}"
        ;;
    esac
    ;;
  */cat)
    if [ "$*" != "$proc/self/cgroup" ]; then
      PATH=$FIXTURE_REAL_PATH exec cat "$@"
    elif [ -s "$group/cgroup.procs" ]; then
      printf '0::%s/workload\n' "$FIXTURE_SCOPE"
    elif [ -s "$FIXTURE_CGROUP$FIXTURE_SCOPE/controller/cgroup.procs" ]; then
      printf '0::%s/controller\n' "$FIXTURE_SCOPE"
    else
      printf '0::%s\n' "$FIXTURE_SCOPE"
    fi
    ;;
  */mkdir)
    PATH=$FIXTURE_REAL_PATH mkdir "$@"
    if [ "$*" = "$group" ]; then
      for control in memory.max memory.high; do echo max > "$group/$control"; done
      echo 1 > "$group/memory.swap.max"
      echo 0 > "$group/memory.oom.group"
      : > "$group/cgroup.kill"
      echo 'populated 0' > "$group/cgroup.events"
      printf 'oom 0\noom_kill 0\noom_group_kill 0\n' > "$group/memory.events"
    fi
    ;;
  */rmdir)
    if [ "$*" = "$group" ]; then
      [ "$(cat "$group/cgroup.events")" = 'populated 0' ]
      [ "${FIXTURE_RMDIR:-ok}" != fail ]
    else
      PATH=$FIXTURE_REAL_PATH exec rmdir "$@"
    fi
    ;;
  */dmesg)
    [ "$*" = --raw ]
    cat "$proc/kmsg"
    ;;
  */stat)
    [ "$*" = "-f -c %T $HOME/eon-run/workload/tmp" ]
    [ -d "$HOME/eon-run/workload/tmp" ]
    printf '%s\n' "${FIXTURE_SCRATCH_FS:-ext2/ext3}"
    ;;
  */setpriv)
    if [ "$1" = --no-new-privs ]; then shift; else [ "$FIXTURE_REFUSE_CAPS" = 0 ] || exit 97; fi
    [ "$1 $2" = '--inh-caps=-all --ambient-caps=-all' ]
    shift 2
    exec "$@"
    ;;
  */bwrap)
    printf '%s\n' "$@" > "$FIXTURE_PROC/bwrap-args"
    [ "${FIXTURE_BWRAP:-ok}" != fail ] || exit 1
    while [ "$1" != -- ]; do shift; done
    shift
    unset XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS DBUS_SYSTEM_BUS_ADDRESS
    code=0
    "$@" || code=$?
    case "${FIXTURE_OOM:-none}" in
      none) exit "$code" ;;
      wrapped) echo '<6>[2.0] replacement boot log' > "$proc/kmsg"; exit "$code" ;;
      unreadable) : > "$proc/vmstat"; exit "$code" ;;
      vm)
        printf '<3>[2.0] Out of memory: Killed process 50 (test) total-vm:100kB\n' >> "$proc/kmsg"
        printf 'oom 0\noom_kill 1\noom_group_kill 0\n' > "$group/memory.events"
        echo 'oom_kill 6' > "$proc/vmstat"
        exit 137
        ;;
    esac
    cg=$FIXTURE_SCOPE/workload
    if [ "${FIXTURE_OOM}" != unknown ]; then
      printf '<6>[2.0] oom-kill:constraint=CONSTRAINT_MEMCG,oom_memcg=%s,task_memcg=%s,task=test,pid=50,uid=1000\n' "$cg" "$cg" >> "$proc/kmsg"
    fi
    {
      echo '<3>[2.1] Memory cgroup out of memory: Killed process 50 (test) total-vm:100kB'
      printf '<6>[2.2] Tasks in %s are going to be killed due to memory.oom.group set\n' "$cg"
      if [ "${FIXTURE_OOM}" = mixed ]; then
        # This VM-wide kill's rate-limited constraint record is missing.
        echo '<3>[2.3] Out of memory: Killed process 51 (test) total-vm:100kB'
      else
        echo '<3>[2.3] Memory cgroup out of memory: Killed process 51 (test) total-vm:100kB'
      fi
    } >> "$proc/kmsg"
    printf 'oom 1\noom_kill 2\noom_group_kill 1\n' > "$group/memory.events"
    echo 'oom_kill 7' > "$proc/vmstat"
    if [ "${FIXTURE_OOM}" = undrained ]; then
      echo 'populated 1' > "$group/cgroup.events"
    elif [ "${FIXTURE_OOM}" = job ]; then
      echo 'populated 1' > "$group/cgroup.events"
      (sleep .15; echo 'populated 0' > "$group/cgroup.events") </dev/null >/dev/null 2>&1 &
    fi
    exit 137
    ;;
esac
