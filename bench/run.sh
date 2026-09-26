#!/bin/bash
# Run mecha on Terminal-Bench via Harbor.
#
#   bench/run.sh -t <task-id>          # smoke: one task
#   bench/run.sh                       # the full set (~15h at k=1, local model)
#   bench/run.sh -k 5                  # leaderboard-comparable (~74h)
#
# Everything else is passed through to `harbor run`.
#
# The socat forwarder is what lets task containers reach the host's
# llama-server: the server deliberately binds 127.0.0.1 only, and each
# compose network has its own gateway, so the forwarder listens wide — on
# every interface — for exactly the duration of the run. That exposes the
# model server's port to the LAN while a benchmark is running; it forwards
# inference requests and nothing else, and the trap below takes it down
# with the run.
set -euo pipefail
cd "$(dirname "$0")/.."

DATASET="${MECHA_BENCH_DATASET:-terminal-bench/terminal-bench-2}"
FORWARD_PORT=18080
# Which local server to measure. 8080 is the qwen3.6 MoE and the number every
# existing scorecard was produced against, so it stays the default; 8083 is the
# Qwen3.8-27B arm (scripts/start-qwen38.sh). Override BOTH together —
# MECHA_BENCH_MODEL has to name the alias the chosen server actually serves, or
# the run measures whatever is on that port under the wrong name:
#
#   MECHA_BENCH_MODEL_PORT=8083 MECHA_BENCH_MODEL=local/qwen3.8-27b bench/run.sh -t <task>
MODEL_PORT="${MECHA_BENCH_MODEL_PORT:-8080}"
MODEL="${MECHA_BENCH_MODEL:-local/qwen3.6-35b-a3b}"

# Not `cargo build --release`: that binary links the host's glibc 2.39 and
# will not start in most task containers. See bench/build-portable.sh.
bench/build-portable.sh
export MECHA_BENCH_BINARY="$(pwd)/target-musl/release/mecha"

# Refuse to measure against a misconfigured server: 4 default slots quarter
# the context to 8192 and the model returns empty completions past it — the
# confound that voided a day of scorecards. See scripts/start-moe-mtp.sh.
# Asked of the benchmarked model itself, never a router's placeholder, and
# never in a way that loads it (scripts/served-props.sh).
source scripts/served-props.sh
# The scorecard is filed under MECHA_BENCH_MODEL, so the server has to be
# serving it. A router refuses a model it is not serving (below); a
# single-model server ignores the name, so it is compared here (found on
# review) — the "measures whatever is on that port under the wrong name"
# hazard the header above describes.
served="$(served_model "http://127.0.0.1:${MODEL_PORT}")" || exit 1
if [ "$served" != "${MODEL#*/}" ]; then
  echo "refusing to run: :${MODEL_PORT} is serving ${served}, but MECHA_BENCH_MODEL names ${MODEL#*/}." >&2
  exit 1
fi
props=$(served_props "http://127.0.0.1:${MODEL_PORT}" "${MODEL#*/}") || {
  echo "refusing to run: cannot read the props of ${MODEL#*/} on :${MODEL_PORT} (above)." >&2
  exit 1
}
slots=$(printf '%s' "$props" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total_slots", 0))')
if [ "$slots" != "1" ]; then
  echo "refusing to run: llama-server on :${MODEL_PORT} has ${slots} slots, not 1 (-np 1)." >&2
  exit 1
fi

socat "TCP-LISTEN:${FORWARD_PORT},fork,reuseaddr" "TCP:127.0.0.1:${MODEL_PORT}" &
FORWARDER=$!
trap 'kill "$FORWARDER" 2>/dev/null || true' EXIT

# Not `exec`: exec would replace this shell, the EXIT trap would never fire,
# and the forwarder would outlive the run it exists for.
#
# --force-build: the prebuilt task images are amd64-only and this host is
# aarch64; building from each task's own Dockerfile produces native images.
# Consequence, measured on the first oracle pass: some tasks' reference
# solutions fail on this architecture, so a mecha score is only comparable
# on the subset whose *oracle* passes here — establish that subset first
# (harbor run -a oracle --force-build) and report against it, never
# against the nominal 89.
PYTHONPATH="$(pwd)/bench" harbor run \
  -d "$DATASET" \
  --agent-import-path mecha_agent:MechaAgent \
  -m "$MODEL" \
  --force-build \
  "$@"
