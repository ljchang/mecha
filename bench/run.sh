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
MODEL_PORT=8080

cargo build --release

# Refuse to measure against a misconfigured server: 4 default slots quarter
# the context to 8192 and the model returns empty completions past it — the
# confound that voided a day of scorecards. See scripts/start-moe-mtp.sh.
slots=$(curl -s "http://127.0.0.1:${MODEL_PORT}/props" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total_slots", 0))')
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
  -m local/qwen3.6-35b-a3b \
  --force-build \
  "$@"
