#!/usr/bin/env bash
# Size Cargo's parallel compiles to the memory the job may use, not to the CPUs it sees: inside a
# sandbox (gVisor on the shared untrusted boxes) nproc reports the host's cores while the memory
# limit is the sandbox's, and one rustc per core on the big crates runs out of memory (rustc
# killed, "exit status: 2" with no diagnostic). Sets CARGO_BUILD_JOBS for the rest of the job.
# Where the memory can't be read (some runners don't expose /proc/meminfo), it sets nothing and
# Cargo keeps its default: sizing is an optimisation and must never fail the job.
# Usage: build-jobs.sh [GB_PER_JOB] (default 3)
set -uo pipefail
per_job=${1:-3}
kb=
if [ -r /proc/meminfo ]; then
  kb=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
fi
# A cgroup limit, where there is one, can be lower than what /proc/meminfo reports.
for f in /sys/fs/cgroup/memory.max /sys/fs/cgroup/memory/memory.limit_in_bytes; do
  if [ -r "$f" ]; then
    limit=$(cat "$f")
    case "$limit" in
      '' | max | *[!0-9]*) ;;
      *) if [ -z "$kb" ] || [ "$((limit / 1024))" -lt "$kb" ]; then kb=$((limit / 1024)); fi ;;
    esac
    break
  fi
done
case "$kb" in
  '' | *[!0-9]*)
    echo "build-jobs: memory not readable here; leaving CARGO_BUILD_JOBS to Cargo's default"
    exit 0
    ;;
esac
cpus=$(nproc 2>/dev/null || echo 1)
jobs=$((kb / 1048576 / per_job))
if [ "$jobs" -lt 1 ]; then jobs=1; fi
if [ "$jobs" -gt "$cpus" ]; then jobs=$cpus; fi
echo "build-jobs: $((kb / 1048576)) GB available, $cpus CPUs -> CARGO_BUILD_JOBS=$jobs"
echo "CARGO_BUILD_JOBS=$jobs" >> "$GITHUB_ENV"
