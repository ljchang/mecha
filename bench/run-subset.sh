#!/bin/bash
# Run mecha over the tasks whose *oracle* passes on this architecture.
#
#   bench/run-subset.sh                 # the 75-task scorecard at k=1
#   bench/run-subset.sh -k 5            # leaderboard-comparable
#
# Everything is passed through to bench/run.sh, and from there to `harbor run`.
#
# Why a subset at all: the prebuilt task images are amd64-only and this host is
# aarch64, so `--force-build` builds each task from its own Dockerfile. Some
# tasks' *reference solutions* then fail here — a score against those measures
# the architecture, not the agent. bench/oracle-arm64-excluded.txt is the list,
# derived from the oracle sweep's result.json rather than typed by hand.
#
# Re-derive it after any dataset bump: a task added upstream is not in either
# file, and would silently be scored without ever having been calibrated.
set -euo pipefail
cd "$(dirname "$0")/.."

EXCLUDED="bench/oracle-arm64-excluded.txt"
[ -f "$EXCLUDED" ] || { echo "missing $EXCLUDED — run the oracle sweep first" >&2; exit 1; }

# Pinned to the exact dataset the oracle sweep measured. `latest` is what
# bench/run.sh defaults to, and it is wrong *here* specifically: the excluded
# list is a claim about 89 particular tasks, so resolving to a newer dataset
# would apply yesterday's calibration to a set that may have gained, dropped or
# changed tasks — and nothing in the output would say so. Re-run the oracle
# sweep and re-derive both files before moving this.
export MECHA_BENCH_DATASET="${MECHA_BENCH_DATASET:-terminal-bench/terminal-bench-2@sha256:c6fc2e2382c1dbae99b2d5ecd2f4f4a60c3c01e0d84642d69b4afd92e99d078b}"

# One -x per excluded task. Comments and blanks are skipped; the names are
# literal, but harbor treats them as globs, so a name containing a glob
# metacharacter would over-match — none do, and this asserts it.
#
# The dataset-qualified check is the one that has actually bitten. harbor
# matches -x against 'terminal-bench/<task>', and a bare '<task>' matches
# nothing — silently, with no warning and exit 0, so the job runs all 89 and
# every artifact still says "subset". Two runs were lost to it. A name with no
# '/' is therefore refused here rather than diagnosed later from a scorecard.
args=()
while read -r task; do
  case "$task" in ''|\#*) continue ;; esac
  case "$task" in *[\*\?\[]*) echo "refusing: '$task' looks like a glob" >&2; exit 1 ;; esac
  case "$task" in */*) ;; *) echo "refusing: '$task' is not dataset-qualified (want 'terminal-bench/$task') — a bare name silently excludes nothing" >&2; exit 1 ;; esac
  args+=(-x "$task")
done < "$EXCLUDED"

echo "excluding $((${#args[@]} / 2)) tasks; verify with bench/check-subset.py once lock.json appears" >&2

# --n-concurrent-agents 1 is load-bearing, not tuning. The host runs ONE
# llama-server with `-np 1` — a single slot holding the whole 32768 context,
# which bench/run.sh refuses to start without. Two agents calling it
# concurrently do not get two slots; they queue, and each switch evicts the
# other's KV prefix, so prompt caching (the thing that makes a 40-turn run
# affordable) collapses and agent timeouts start firing on the queue rather
# than on the work. So: four trials in flight, but only one of them talking
# to the model at a time. The build and verify phases are CPU work with no
# model in them, and those still overlap.
exec bench/run.sh \
  --job-name "mecha-arm64-subset-$(date +%Y-%m-%d__%H-%M-%S)" \
  -o jobs/mecha-arm64-subset \
  -n 4 \
  --n-concurrent-agents 1 \
  "${args[@]}" \
  "$@"
