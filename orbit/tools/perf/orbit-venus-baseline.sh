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

Required environment for benchmark runs:
  ORB_BASELINE_CPUS       taskset CPU list, for example 9-15
  ORB_BASELINE_GRID       expected PTY grid as "ROWS COLS"

The run opens native Venus windows; use an isolated display.
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
    local stats max extracted pidstat_sample failure
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
    *)
        usage >&2
        exit 2
        ;;
    esac
}

main "$@"
