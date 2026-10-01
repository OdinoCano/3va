#!/bin/sh
# hook-limits.sh — sourced by the git hooks (.cargo-husky/hooks/*).
#
# The hooks run cargo (clippy, the whole test suite, fuzz builds). Left
# alone, cargo uses every core and as much memory as linking a V8-sized
# workspace wants, in the same session as the desktop: on a 32-core / 30 GB
# machine that pushed memory pressure high enough for systemd-oomd to kill
# the whole graphical session, twice.
#
# This sizes the work to the machine that runs it instead:
#   - memory cap  = 40 % of RAM, at least 4 GB where the machine has it
#                   (soft limit at 3/4 of that)
#   - parallelism = one job per 3 GB of the soft limit, at most half the cores
# and, where a systemd user session exists, runs each heavy command in its
# own scope with those limits, so the worst case is the hook being stopped —
# never the session. Without systemd it still limits jobs and lowers priority.
# Slower on purpose.
#
# Override per machine with:
#   VVVA_HOOK_JOBS=<n>          parallel cargo jobs / test threads
#   VVVA_HOOK_MEM_MAX_MB=<mb>   hard memory cap for a hook command

_hook_cores() {
    if command -v nproc >/dev/null 2>&1; then
        nproc
    elif command -v sysctl >/dev/null 2>&1; then
        sysctl -n hw.ncpu 2>/dev/null || echo 2
    else
        echo 2
    fi
}

_hook_mem_mb() {
    if [ -r /proc/meminfo ]; then
        awk '/^MemTotal:/ { print int($2 / 1024) }' /proc/meminfo
    elif command -v sysctl >/dev/null 2>&1; then
        bytes=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
        echo $((bytes / 1048576))
    else
        echo 0
    fi
}

hook_limits_init() {
    _cores=$(_hook_cores)
    _mem=$(_hook_mem_mb)
    # Unknown memory size: assume a small machine.
    [ "${_mem:-0}" -gt 0 ] 2>/dev/null || _mem=4096

    # 40 % of RAM, but never less than one link step needs (~4 GB) when the
    # machine has it: a cap below that would make the hook unable to pass.
    _cap=$((_mem * 40 / 100))
    _floor=$((_mem - 1024))
    [ "$_floor" -gt 4096 ] && _floor=4096
    [ "$_cap" -lt "$_floor" ] && _cap=$_floor
    HOOK_MEM_MAX_MB=${VVVA_HOOK_MEM_MAX_MB:-$_cap}
    HOOK_MEM_HIGH_MB=$((HOOK_MEM_MAX_MB * 3 / 4))

    _jobs=$((HOOK_MEM_HIGH_MB / 3072))
    _half=$((_cores / 2))
    [ "$_half" -lt 1 ] && _half=1
    [ "$_jobs" -lt 1 ] && _jobs=1
    [ "$_jobs" -gt "$_half" ] && _jobs=$_half
    HOOK_JOBS=${VVVA_HOOK_JOBS:-$_jobs}

    # cargo and the test harness both read these.
    CARGO_BUILD_JOBS=$HOOK_JOBS
    RUST_TEST_THREADS=$HOOK_JOBS
    export CARGO_BUILD_JOBS RUST_TEST_THREADS

    HOOK_SCOPE=0
    if command -v systemd-run >/dev/null 2>&1 \
        && systemd-run --user --scope --quiet --collect true >/dev/null 2>&1; then
        HOOK_SCOPE=1
    fi

    if [ "$HOOK_SCOPE" = 1 ]; then
        _how="systemd scope, ${HOOK_MEM_MAX_MB} MB cap"
    else
        _how="no systemd user session: jobs and priority only"
    fi
    echo "[hook] ${_cores} cores, ${_mem} MB RAM -> ${HOOK_JOBS} parallel job(s); ${_how}"
}

# run_limited <command...>: run a heavy command within the limits above.
run_limited() {
    if [ "${HOOK_SCOPE:-0}" = 1 ]; then
        systemd-run --user --scope --quiet --collect \
            -p "MemoryHigh=${HOOK_MEM_HIGH_MB}M" \
            -p "MemoryMax=${HOOK_MEM_MAX_MB}M" \
            -p MemorySwapMax=1G \
            nice -n 10 "$@"
    else
        nice -n 10 "$@"
    fi
}
