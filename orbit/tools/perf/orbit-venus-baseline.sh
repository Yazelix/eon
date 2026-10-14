#!/usr/bin/env bash
set -Eeuo pipefail
export LANG=C.UTF-8 LC_ALL=C.UTF-8
readonly SCRIPT_NAME=${0##*/}
readonly PALETTE='000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff'
readonly LOAD_LIMIT=2.0
readonly COMPOSITOR_CPU_LIMIT=10.0

orbit_pid=
venus_pid=
sampler_pid=
socket_root=

usage() {
    cat <<EOF
Usage:
  $SCRIPT_NAME --self-check
  $SCRIPT_NAME throughput ORBIT_BIN VENUS_BIN VTEBENCH_DIR EVIDENCE_PARENT
  $SCRIPT_NAME lifecycle  ORBIT_BIN VENUS_BIN EVIDENCE_PARENT

Required environment for benchmark runs:
  ORB_BASELINE_CPUS       taskset CPU list, for example 9-15
  ORB_BASELINE_GRID       expected PTY grid as "ROWS COLS"

Lifecycle also requires ORB_BASELINE_NVIM and ORB_BASELINE_YAZI.
The run opens native Venus and resize-stimulus windows; leave the desktop idle.
EOF
}

die() {
    printf '%s: %s\n' "$SCRIPT_NAME" "$*" >&2
    exit 1
}

cleanup() {
    if [[ -n ${sampler_pid:-} ]]; then
        kill "$sampler_pid" 2>/dev/null || true
        wait "$sampler_pid" 2>/dev/null || true
        sampler_pid=
    fi
    if [[ -n ${venus_pid:-} ]]; then
        kill "$venus_pid" 2>/dev/null || true
        wait "$venus_pid" 2>/dev/null || true
        venus_pid=
    fi
    if [[ -n ${orbit_pid:-} ]]; then
        kill "$orbit_pid" 2>/dev/null || true
        wait "$orbit_pid" 2>/dev/null || true
        orbit_pid=
    fi
    if [[ -n ${socket_root:-} ]]; then
        rm -f -- "$socket_root/orbit.sock"
        rmdir -- "$socket_root" 2>/dev/null || true
        socket_root=
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

need_command() {
    command -v "$1" >/dev/null || die "missing command: $1"
}

need_file() {
    [[ -x $1 ]] || die "not executable: $1"
}

wait_for_file() {
    local path=$1 attempts=${2:-200}
    local attempt
    for ((attempt = 0; attempt < attempts; attempt++)); do
        [[ -e $path ]] && return 0
        sleep 0.05
    done
    return 1
}

stats_values() {
    local context=$1 value values=()
    while read -r value _; do
        [[ -z ${value:-} ]] && continue
        [[ $value =~ ^[0-9]+([.][0-9]+)?$ ]] || die "$context contains non-numeric value: $value"
        values+=("$value")
    done
    ((${#values[@]} > 0)) || die "$context has no values"
    mapfile -t values < <(printf '%s\n' "${values[@]}" | sort -n)
    local count=${#values[@]}
    local p50=$(((50 * count + 99) / 100 - 1))
    local p95=$(((95 * count + 99) / 100 - 1))
    printf '%d %s %s %s\n' "$count" "${values[$p50]}" "${values[$p95]}" "${values[$((count - 1))]}"
}

pidstat_max_percent() {
    local pid=$1 file=${2:--}
    awk -v pid="$pid" '
        $3 == pid {
            seen = 1
            if ($8 !~ /^[0-9]+([.][0-9]+)?$/) invalid = 1
            else if ($8 + 0 > max) max = $8 + 0
        }
        END { if (!seen || invalid) exit 1; print max + 0 }
    ' "$file"
}

assert_dat_columns() {
    local path=$1 expected=$2 context=$3 actual
    actual=$(head -n 1 "$path" | wc -w)
    ((expected > 0 && actual == expected)) ||
        die "$context produced $actual of $expected benchmark columns"
}

self_check() {
    local stats max extracted aggregated pidstat_sample pidstat_phase_sample failure
    local check_parent evidence_root socket_check_root socket_path
    stats=$(printf '5\n1\n4\n2\n3\n' | stats_values 'percentile self-check')
    [[ $stats == '5 3 5 5' ]] || die "percentile self-check failed: $stats"
    if failure=$(printf 'oops\n' | stats_values 'non-numeric self-check' 2>&1); then
        die 'non-numeric statistics self-check failed'
    fi
    [[ $failure == *'non-numeric self-check contains non-numeric value: oops' ]] || die "unexpected statistics error: $failure"
    if failure=$(stats_values 'empty self-check' </dev/null 2>&1); then
        die 'empty statistics self-check failed'
    fi
    [[ $failure == *'empty self-check has no values' ]] || die "unexpected statistics error: $failure"
    assert_dat_columns <(printf 'left right\n1 2\n') 2 'DAT self-check'
    if failure=$(assert_dat_columns <(printf '\n') 2 'DAT self-check' 2>&1); then
        die 'incomplete DAT self-check failed'
    fi
    [[ $failure == *'DAT self-check produced 0 of 2 benchmark columns' ]] || die "unexpected DAT error: $failure"
    check_parent=$(mktemp -d)
    evidence_root=$(cd "$check_parent" && new_evidence_root evidence self-check)
    if [[ $evidence_root != /* ]]; then
        rmdir "$check_parent/$evidence_root" "$check_parent/evidence" "$check_parent"
        die "evidence root is not absolute: $evidence_root"
    fi
    rmdir "$evidence_root" "$check_parent/evidence"
    if ! socket_root=$(XDG_RUNTIME_DIR="$check_parent" new_socket_root); then
        rmdir "$check_parent"
        die 'socket root self-check failed'
    fi
    socket_check_root=$socket_root
    socket_path="$socket_root/orbit.sock"
    ((${#socket_path} < 108)) || die "socket path is too long: $socket_path"
    : >"$socket_path"
    cleanup
    [[ ! -e $socket_check_root ]] || die "socket cleanup self-check failed: $socket_check_root"
    rmdir "$check_parent"
    pidstat_sample=$'# Time UID PID %usr %system %guest %wait %CPU CPU Command\n00:00:01 1000 42 1.00 2.00 0.00 0.00 3.00 9 compositor\n00:00:02 1000 42 4.00 5.00 0.00 0.00 9.00 2 compositor'
    max=$(pidstat_max_percent 42 <<<"$pidstat_sample")
    [[ $max == 9 ]] || die "pidstat column self-check failed: $max"
    if pidstat_max_percent 99 <<<"$pidstat_sample" >/dev/null; then
        die 'missing pidstat PID self-check failed'
    fi
    if pidstat_max_percent 42 <<<"${pidstat_sample/9.00/oops}" >/dev/null; then
        die 'non-numeric pidstat CPU self-check failed'
    fi
    pidstat_phase_sample=$'00:00:01 1000 42 0 0 0 0 1 9 0 0 10 100 0 process-a\n00:00:01 1000 43 0 0 0 0 2 8 0 0 20 200 0 process-b'
    aggregated=$(phase_samples <(printf '%s\n' "$pidstat_phase_sample") 1)
    [[ $aggregated == '00:00:01 3 300' ]] || die "phase aggregation self-check failed: $aggregated"
    if phase_samples <(printf '%s\n' "${pidstat_phase_sample/ 1 9 / oops 9 }") 1 >/dev/null; then
        die 'non-numeric phase CPU self-check failed'
    fi
    if phase_samples <(printf '%s\n' "${pidstat_phase_sample/ 100 0 process-a/ nope 0 process-a}") 1 >/dev/null; then
        die 'non-numeric phase RSS self-check failed'
    fi
    extracted=$(printf 'left right\n1 2\n' | tail -n +2 | awk -v column=2 '{ print $column }')
    [[ $extracted == 2 ]] || die "DAT column self-check failed: $extracted"
    [[ $(awk 'BEGIN { print (1.99 <= 2.0) ? "yes" : "no" }') == yes ]] || die "numeric gate self-check failed"
    printf 'self-check passed\n'
}

validate_run_environment() {
    [[ -n ${ORB_BASELINE_CPUS:-} ]] || die 'ORB_BASELINE_CPUS is required'
    [[ ${ORB_BASELINE_GRID:-} =~ ^[1-9][0-9]*\ [1-9][0-9]*$ ]] || die 'ORB_BASELINE_GRID must be "ROWS COLS"'
    [[ -n ${XDG_RUNTIME_DIR:-} && -n ${WAYLAND_DISPLAY:-} ]] || die 'a native Wayland session is required'
    read -r expected_rows expected_cols <<<"$ORB_BASELINE_GRID"
    readonly expected_rows expected_cols
    need_command awk
    need_command date
    need_command pidstat
    need_command pgrep
    need_command sha256sum
    need_command ss
    need_command taskset
    compositor_pid=${ORB_BASELINE_COMPOSITOR_PID:-$(pgrep -xo cosmic-comp || true)}
    [[ $compositor_pid =~ ^[1-9][0-9]*$ ]] || die 'set ORB_BASELINE_COMPOSITOR_PID to the compositor PID'
    readonly compositor_pid
}

new_evidence_root() {
    local parent=$1 mode=$2
    [[ -z $parent || $parent == /* ]] || parent=$PWD/$parent
    mkdir -p "$parent"
    mktemp -d "$parent/orb-1sd-$mode.XXXXXX"
}

new_socket_root() {
    mktemp -d "$XDG_RUNTIME_DIR/orb-1sd.XXXXXX"
}

record_environment() {
    local root=$1 orbit=$2 venus=$3 vtebench=${4:-}
    local harness
    harness=$(readlink -f "$0")
    cp -- "$harness" "$root/harness.sh"
    {
        printf 'recorded_at=%s\n' "$(date --iso-8601=seconds)"
        printf 'harness=%s\norbit=%s\nvenus=%s\n' "$harness" "$(readlink -f "$orbit")" "$(readlink -f "$venus")"
        sha256sum "$harness" "$orbit" "$venus"
        printf 'cpus=%s\ngrid=%s %s\n' "$ORB_BASELINE_CPUS" "$expected_rows" "$expected_cols"
        printf 'wayland_display=%s\nxdg_runtime_dir=%s\n' "$WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR"
        printf 'term=eon\nlocale=C.UTF-8\n'
        uname -a
        lscpu
        free -b
        cosmic-randr list 2>&1 || true
        printf 'loadavg=' && cat /proc/loadavg
        printf 'governor=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || true)"
        printf 'energy_preference=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null || true)"
        printf 'ac_online=%s\n' "$(cat /sys/class/power_supply/AC*/online 2>/dev/null | head -n 1 || true)"
        if [[ -n $vtebench ]]; then
            printf 'vtebench_revision=%s\n' "$(git -C "$vtebench" rev-parse HEAD)"
            printf 'vtebench_benchmarks_tree=%s\n' "$(git -C "$vtebench" rev-parse HEAD:benchmarks)"
            sha256sum "$vtebench/target/release/vtebench"
        fi
    } >"$root/environment.txt"
}

preflight() {
    local root=$1
    local sample="$root/compositor-preflight.txt"
    pidstat -h -u -p "$compositor_pid" 1 5 >"$sample"
    local load_one compositor_max
    load_one=$(cut -d' ' -f1 /proc/loadavg)
    compositor_max=$(pidstat_max_percent "$compositor_pid" "$sample") || die "pidstat did not report numeric CPU for compositor PID $compositor_pid"
    printf 'load_one=%s\ncompositor_max_cpu=%s\n' "$load_one" "$compositor_max" >"$root/preflight.txt"
    awk -v value="$load_one" -v limit="$LOAD_LIMIT" 'BEGIN { exit !(value <= limit) }' ||
        die "one-minute load $load_one exceeds $LOAD_LIMIT"
    awk -v value="$compositor_max" -v limit="$COMPOSITOR_CPU_LIMIT" 'BEGIN { exit !(value <= limit) }' ||
        die "compositor CPU $compositor_max exceeds $COMPOSITOR_CPU_LIMIT"
}

assert_grid() {
    local path=$1
    local actual
    actual=$(<"$path")
    [[ $actual == "$expected_rows $expected_cols" ]] || die "grid changed: expected $expected_rows $expected_cols, got $actual"
}

venus_connected() {
    local socket=$1
    kill -0 "$venus_pid" 2>/dev/null &&
        ss -xnp state connected 2>/dev/null | grep -F "$socket" >/dev/null
}

launch_venus() {
    local venus=$1 socket=$2 log=$3
    taskset -c "$ORB_BASELINE_CPUS" env \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
        XDG_SESSION_TYPE=wayland TERM=eon \
        "$venus" --no-decorations --background-opacity 0.8 --background-blur "$socket" >"$log" 2>&1 &
    venus_pid=$!
    for _ in {1..1000}; do
        venus_connected "$socket" && return
        kill -0 "$venus_pid" 2>/dev/null || die 'Venus exited before attachment'
        sleep 0.01
    done
    die 'Venus did not connect in 10 seconds'
}

run_vtebench_once() {
    local root=$1 condition=$2 repetition=$3 orbit=$4 venus=$5 vtebench=$6 corpus_count=$7
    local run_root
    run_root=$(mktemp -d "$root/$condition-$repetition.XXXXXX")
    socket_root=$(new_socket_root)
    local socket="$socket_root/orbit.sock"
    local ready="$run_root/ready" go="$run_root/go"
    local pre_grid="$run_root/pre-grid.txt" run_grid="$run_root/run-grid.txt"
    local dat="$run_root/results.dat"

    taskset -c "$ORB_BASELINE_CPUS" env \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
        XDG_SESSION_TYPE=wayland TERM=eon \
        "$orbit" serve "$socket" --ansi-palette-v1 "$PALETTE" -- /bin/bash -c '
            printf "\033]0;orb-1sd %s %s\007" "$1" "$2"
            sleep 2
            stty size >"$3"
            touch "$4"
            until [[ -e $5 ]]; do sleep 0.05; done
            stty size >"$6"
            cd "$7"
            exec target/release/vtebench --silent --warmup 1 --min-bytes 1048576 --max-samples 5 --max-secs 10 --dat "$8"
        ' bash "$condition" "$repetition" "$pre_grid" "$ready" "$go" "$run_grid" "$vtebench" "$dat" \
        >"$run_root/orbit.log" 2>&1 &
    orbit_pid=$!
    for _ in {1..200}; do [[ -S $socket ]] && break; sleep 0.05; done
    [[ -S $socket ]] || die 'Orbit socket did not appear'
    launch_venus "$venus" "$socket" "$run_root/venus.log"
    wait_for_file "$ready" || die 'Venus did not establish the PTY grid'
    assert_grid "$pre_grid"

    if [[ $condition == detached ]]; then
        kill "$venus_pid"
        wait "$venus_pid" 2>/dev/null || true
        venus_pid=
        pidstat -h -u -r -p "$orbit_pid" 1 >"$run_root/pidstat.txt" &
    else
        pidstat -h -u -r -p "$orbit_pid,$venus_pid" 1 >"$run_root/pidstat.txt" &
    fi
    sampler_pid=$!

    local start_ns end_ns completed=no orbit_status=timeout
    start_ns=$(date +%s%N)
    touch "$go"
    for _ in {1..1800}; do
        if ! kill -0 "$orbit_pid" 2>/dev/null; then
            if wait "$orbit_pid"; then orbit_status=0; else orbit_status=$?; fi
            orbit_pid=
            [[ $orbit_status == 0 && -s $dat ]] && completed=yes
            break
        fi
        sleep 0.1
    done
    end_ns=$(date +%s%N)
    cleanup
    [[ $completed == yes ]] || die "$condition repetition $repetition failed or timed out (status: $orbit_status)"
    assert_grid "$run_grid"
    assert_dat_columns "$dat" "$corpus_count" "$condition repetition $repetition"
    printf 'condition=%s\nrepetition=%s\nwall_ms=%s\ngrid=%s %s\ncompleted=yes\n' \
        "$condition" "$repetition" "$(((end_ns - start_ns) / 1000000))" "$expected_rows" "$expected_cols" \
        >"$run_root/run.txt"
    printf 'completed %s repetition %s\n' "$condition" "$repetition"
}

dat_values() {
    local root=$1 condition=$2 column=$3
    local file
    while IFS= read -r -d '' file; do
        tail -n +2 "$file" | awk -v column="$column" '$column != "_" { print $column }'
    done < <(find "$root" -path "*/$condition-*/*" -name results.dat -print0 | sort -z)
}

summarize_throughput() {
    local root=$1
    local first
    first=$(find "$root" -path '*/detached-*/*' -name results.dat -print | sort | head -n 1)
    [[ -n $first ]] || die 'no detached DAT results found'
    local summary="$root/summary.tsv"
    printf 'benchmark\tdet_n\tdet_p50_ms\tdet_p95_ms\tatt_n\tatt_p50_ms\tatt_p95_ms\tp50_delta_pct\tp95_delta_pct\n' >"$summary"
    local index=1 name
    for name in $(head -n 1 "$first"); do
        local detached attached
        detached=$(dat_values "$root" detached "$index" | stats_values "$name detached samples")
        attached=$(dat_values "$root" attached "$index" | stats_values "$name attached samples")
        local dn d50 d95 an a50 a95
        read -r dn d50 d95 _ <<<"$detached"
        read -r an a50 a95 _ <<<"$attached"
        ((dn == 15 && an == 15)) || die "$name has $dn detached and $an attached samples; expected 15 each"
        local delta50 delta95
        delta50=$(awk -v d="$d50" -v a="$a50" 'BEGIN { if (d == 0) print "unbounded"; else printf "%.1f", 100 * (a - d) / d }')
        delta95=$(awk -v d="$d95" -v a="$a95" 'BEGIN { if (d == 0) print "unbounded"; else printf "%.1f", 100 * (a - d) / d }')
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$name" "$dn" "$d50" "$d95" "$an" "$a50" "$a95" "$delta50" "$delta95" >>"$summary"
        index=$((index + 1))
    done
    cat "$summary"
}

run_throughput() {
    local orbit=$1 venus=$2 vtebench=$3 parent=$4
    need_command git
    need_file "$orbit"
    need_file "$venus"
    need_file "$vtebench/target/release/vtebench"
    [[ -d $vtebench/benchmarks ]] || die "missing vtebench corpus: $vtebench/benchmarks"
    git -C "$vtebench" rev-parse --verify HEAD >/dev/null 2>&1 || die "not a Git checkout: $vtebench"
    [[ -z $(git -C "$vtebench" status --porcelain --untracked-files=all) ]] || die 'vtebench checkout has tracked or untracked changes'
    local corpus_count
    corpus_count=$(find -L "$vtebench/benchmarks" -type f -name benchmark -printf '%h\n' |
        awk -F/ '{ print $NF }' | sort -u | wc -l) || die 'unable to enumerate vtebench corpus'
    ((corpus_count > 0)) || die 'vtebench corpus has no benchmarks'
    local root
    root=$(new_evidence_root "$parent" throughput)
    printf 'evidence_root=%s\n' "$root"
    record_environment "$root" "$orbit" "$venus" "$vtebench"
    preflight "$root"
    local condition repetition
    for condition in detached attached; do
        for repetition in 1 2 3; do
            run_vtebench_once "$root" "$condition" "$repetition" "$orbit" "$venus" "$vtebench" "$corpus_count"
        done
    done
    summarize_throughput "$root"
}

lifecycle_child() {
    local state=$1 nvim=$2 yazi=$3
    wait_for() { until [[ -e $1 ]]; do sleep 0.05; done; }
    printf '\033]0;orb-1sd lifecycle\007'
    sleep 2
    stty size >"$state/initial-grid.txt"
    touch "$state/boot.ready"

    wait_for "$state/idle.go"
    printf '\033]0;orb-1sd idle\007'
    sleep 30
    touch "$state/idle.done"

    wait_for "$state/shell.go"
    printf '\033[2J\033[H\033]0;orb-1sd shell\007\033[1;36mOrbit shell baseline\033[0m\nASCII abc XYZ 0123 | Unicode λ界🙂 é\n'
    sleep 10
    touch "$state/shell.done"

    wait_for "$state/nvim.go"
    printf '\033]0;orb-1sd Neovim\007'
    timeout --signal=TERM --kill-after=1s 10s "$nvim" --clean || true
    touch "$state/nvim.done"

    wait_for "$state/yazi.go"
    printf '\033]0;orb-1sd Yazi\007'
    timeout --signal=TERM --kill-after=1s 10s "$yazi" || true
    touch "$state/yazi.done"

    wait_for "$state/sustained.go"
    printf '\033]0;orb-1sd sustained\007'
    timeout --signal=TERM --kill-after=1s 20s /bin/sh -c 'while :; do seq 1 20000; done' || true
    printf '\033[2J\033[HORB1SD_FINAL\n\033]0;ORB1SD_FINAL\007'
    touch "$state/sustained.done"

    wait_for "$state/resize.go"
    trap 'printf "%s " "$(date +%s%N)" >>"$state/resize-events.txt"; stty size >>"$state/resize-events.txt"' WINCH
    touch "$state/resize.ready"
    wait_for "$state/resize.stop"
    stty size >"$state/resize-final-grid.txt"
    trap - WINCH
    touch "$state/resize.done"

    wait_for "$state/detach.go"
    touch "$state/detach.ready"
    wait_for "$state/detach.emit"
    printf '\033[2J\033[HORB1SD_DETACHED_FINAL\n\033]0;ORB1SD_DETACHED_FINAL\007'
    touch "$state/detach.emitted"
    wait_for "$state/detach.finish"
    stty size >"$state/detach-final-grid.txt"
    touch "$state/detach.done"
}
export -f lifecycle_child

sample_phase() {
    local root=$1 state=$2 phase=$3 timeout_seconds=$4
    pidstat -h -u -r -p "$orbit_pid,$venus_pid" 1 >"$root/$phase-pidstat.txt" &
    sampler_pid=$!
    local start_ns end_ns completed=no
    start_ns=$(date +%s%N)
    touch "$state/$phase.go"
    local attempt
    for ((attempt = 0; attempt < timeout_seconds * 20; attempt++)); do
        if [[ -e $state/$phase.done ]]; then completed=yes; break; fi
        sleep 0.05
    done
    end_ns=$(date +%s%N)
    kill "$sampler_pid" 2>/dev/null || true
    wait "$sampler_pid" 2>/dev/null || true
    sampler_pid=
    printf 'phase=%s\nwall_ms=%s\ncompleted=%s\n' "$phase" "$(((end_ns - start_ns) / 1000000))" "$completed" \
        >"$root/$phase-run.txt"
    [[ $completed == yes ]] || die "$phase did not complete"
}

phase_samples() {
    local file=$1 limit=$2
    awk '$3 ~ /^[0-9]+$/ {
        if ($8 !~ /^[0-9]+([.][0-9]+)?$/ || $13 !~ /^[0-9]+$/) {
            invalid = 1
            next
        }
        cpu[$1] += $8
        rss[$1] += $13
        rows[$1]++
        if (!seen[$1]++) order[++count] = $1
    }
    END {
        if (invalid) exit 1
        for (i = 1; i <= count; i++) {
            time = order[i]
            if (rows[time] == 2) print time, cpu[time], rss[time]
        }
    }' "$file" | head -n "$limit"
}

summarize_phase() {
    local root=$1 phase=$2 limit=$3 cpu_limit=$4 rss_limit_mib=$5
    local samples="$root/$phase-samples.tsv"
    phase_samples "$root/$phase-pidstat.txt" "$limit" >"$samples" ||
        die "$phase pidstat contains non-numeric CPU or RSS"
    local count cpu_stats p50 p95 max peak_rss first_rss
    count=$(wc -l <"$samples")
    ((count == limit)) || die "$phase has $count complete process samples, expected $limit"
    cpu_stats=$(awk '{ print $2 }' "$samples" | stats_values "$phase CPU samples")
    read -r _ p50 p95 max <<<"$cpu_stats"
    peak_rss=$(awk '$3 > max { max = $3 } END { print max + 0 }' "$samples")
    first_rss=$(awk 'NR == 1 { print $3 }' "$samples")
    printf '%s\t%s\t%s\t%s\t%s\t%.1f\t%.1f\n' "$phase" "$count" "$p50" "$p95" "$max" \
        "$(awk -v value="$peak_rss" 'BEGIN { print value / 1024 }')" \
        "$(awk -v value="$first_rss" 'BEGIN { print value / 1024 }')" >>"$root/lifecycle-summary.tsv"
    awk -v value="$p95" -v limit="$cpu_limit" 'BEGIN { exit !(value <= limit) }' || die "$phase CPU p95 $p95 exceeds $cpu_limit"
    awk -v value="$peak_rss" -v limit="$((rss_limit_mib * 1024))" 'BEGIN { exit !(value <= limit) }' ||
        die "$phase RSS exceeds ${rss_limit_mib}MiB"
}

run_lifecycle() {
    local orbit=$1 venus=$2 parent=$3
    need_file "$orbit"
    need_file "$venus"
    [[ -n ${ORB_BASELINE_NVIM:-} && -n ${ORB_BASELINE_YAZI:-} ]] || die 'ORB_BASELINE_NVIM and ORB_BASELINE_YAZI are required'
    need_file "$ORB_BASELINE_NVIM"
    need_file "$ORB_BASELINE_YAZI"
    need_command foot
    need_command timeout
    local root state socket foot
    foot=$(command -v foot)
    root=$(new_evidence_root "$parent" lifecycle)
    state="$root/child"
    mkdir -p "$state"
    printf 'evidence_root=%s\n' "$root"
    record_environment "$root" "$orbit" "$venus"
    {
        printf 'nvim=%s\nyazi=%s\nfoot=%s\n' \
            "$(readlink -f "$ORB_BASELINE_NVIM")" "$(readlink -f "$ORB_BASELINE_YAZI")" "$(readlink -f "$foot")"
        sha256sum "$ORB_BASELINE_NVIM" "$ORB_BASELINE_YAZI" "$foot"
        "$ORB_BASELINE_NVIM" --version | head -n 2
        "$ORB_BASELINE_YAZI" --version | head -n 1
        "$foot" --version | head -n 1
    } >"$root/workload-versions.txt"
    preflight "$root"
    socket_root=$(new_socket_root)
    socket="$socket_root/orbit.sock"

    taskset -c "$ORB_BASELINE_CPUS" env \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
        XDG_SESSION_TYPE=wayland TERM=eon \
        "$orbit" serve "$socket" --ansi-palette-v1 "$PALETTE" -- /bin/bash -c \
        'lifecycle_child "$1" "$2" "$3"' bash "$state" "$ORB_BASELINE_NVIM" "$ORB_BASELINE_YAZI" \
        >"$root/orbit.log" 2>&1 &
    orbit_pid=$!
    for _ in {1..200}; do [[ -S $socket ]] && break; sleep 0.05; done
    [[ -S $socket ]] || die 'Orbit socket did not appear'
    launch_venus "$venus" "$socket" "$root/venus-initial.log"
    wait_for_file "$state/boot.ready" || die 'lifecycle child did not become ready'
    assert_grid "$state/initial-grid.txt"

    sample_phase "$root" "$state" idle 35
    sample_phase "$root" "$state" shell 15
    sample_phase "$root" "$state" nvim 15
    sample_phase "$root" "$state" yazi 15
    sample_phase "$root" "$state" sustained 25

    printf 'phase\tsamples\tcpu_p50\tcpu_p95\tcpu_max\tpeak_rss_mib\tfirst_rss_mib\n' >"$root/lifecycle-summary.tsv"
    summarize_phase "$root" idle 30 5 128
    summarize_phase "$root" shell 10 100 192
    summarize_phase "$root" nvim 10 100 192
    summarize_phase "$root" yazi 10 100 192
    summarize_phase "$root" sustained 20 200 192
    local sustained_first sustained_peak
    sustained_first=$(awk 'NR == 1 { print $3 }' "$root/sustained-samples.tsv")
    sustained_peak=$(awk '$3 > max { max = $3 } END { print max + 0 }' "$root/sustained-samples.tsv")
    (((sustained_peak - sustained_first) <= 32 * 1024)) || die 'sustained RSS grew by more than 32MiB'

    touch "$state/resize.go"
    wait_for_file "$state/resize.ready" || die 'resize phase did not become ready'
    local cycle
    for cycle in {1..50}; do
        env XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$WAYLAND_DISPLAY" \
            "$foot" --title=orb-1sd-resize /bin/sh -c 'sleep 0.12' >"$root/foot-$cycle.log" 2>&1
        sleep 0.05
    done
    sleep 0.1
    touch "$state/resize.stop"
    wait_for_file "$state/resize.done" || die 'resize phase did not finish'
    assert_grid "$state/resize-final-grid.txt"
    local event_count restore_start restore_end restore_ms
    event_count=$(wc -l <"$state/resize-events.txt")
    ((event_count >= 2)) || die "resize emitted only $event_count PTY size events"
    restore_start=$(tail -n 2 "$state/resize-events.txt" | head -n 1 | awk '{ print $1 }')
    restore_end=$(tail -n 1 "$state/resize-events.txt" | awk '{ print $1 }')
    restore_ms=$(((restore_end - restore_start) / 1000000))
    ((restore_ms <= 1000)) || die "resize restoration took ${restore_ms}ms"
    printf 'cycles=50\nevents=%s\nrestore_pair_ms=%s\nfinal_grid=%s %s\n' \
        "$event_count" "$restore_ms" "$expected_rows" "$expected_cols" >"$root/resize-summary.txt"

    touch "$state/detach.go"
    wait_for_file "$state/detach.ready" || die 'detach phase did not become ready'
    kill "$venus_pid"
    wait "$venus_pid" 2>/dev/null || true
    venus_pid=
    kill -0 "$orbit_pid" 2>/dev/null || die 'Orbit died with Venus'
    [[ -S $socket ]] || die 'Orbit socket disappeared with Venus'
    touch "$state/detach.emit"
    wait_for_file "$state/detach.emitted" || die 'detached marker was not emitted'
    local reattach_start reattach_end
    reattach_start=$(date +%s%N)
    launch_venus "$venus" "$socket" "$root/venus-reattach.log"
    reattach_end=$(date +%s%N)
    local reattach_ms=$(((reattach_end - reattach_start) / 1000000))
    ((reattach_ms <= 2000)) || die "reattachment took ${reattach_ms}ms"
    sleep 0.5
    touch "$state/detach.finish"
    wait_for_file "$state/detach.done" || die 'detach phase did not finish'
    assert_grid "$state/detach-final-grid.txt"
    printf 'orbit_survived=yes\nsocket_ms=%s\nfinal_grid=%s %s\nvisible_coherence=unavailable_on_native_wayland\n' \
        "$reattach_ms" "$expected_rows" "$expected_cols" >"$root/detach-summary.txt"

    local orbit_status
    if wait "$orbit_pid"; then orbit_status=0; else orbit_status=$?; fi
    orbit_pid=
    ((orbit_status == 0)) || die "Orbit lifecycle process failed (status: $orbit_status)"
    cleanup
    cat "$root/lifecycle-summary.tsv"
    cat "$root/resize-summary.txt"
    cat "$root/detach-summary.txt"
}

main() {
    case ${1:-} in
    --self-check)
        [[ $# == 1 ]] || die '--self-check takes no arguments'
        self_check
        ;;
    throughput)
        [[ $# == 5 ]] || { usage >&2; exit 2; }
        validate_run_environment
        run_throughput "$2" "$3" "$4" "$5"
        ;;
    lifecycle)
        [[ $# == 4 ]] || { usage >&2; exit 2; }
        validate_run_environment
        run_lifecycle "$2" "$3" "$4"
        ;;
    *)
        usage >&2
        exit 2
        ;;
    esac
}

main "$@"
