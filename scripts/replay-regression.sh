#!/bin/bash
# Replay pinned recorded sessions against the current build and fail on any
# divergence — the standing harness-regression check the replay driver was
# built for: every pinned session is a free regression case, recorded once
# from real work instead of hand-written.
#
#   scripts/replay-regression.sh              # replay every pinned session
#   scripts/replay-regression.sh <id> [...]   # replay just these
#
# Pins live in ~/.mecha/regression-sessions.txt, one session id per line
# (`#` comments allowed) — machine-local on purpose: transcripts are
# personal data and do not belong in the repository. Add a pin by recording
# a session that uses only builtin tools (an MCP surface makes a pin break
# whenever a server is rewired, which is drift, not regression):
#
#   mecha run -p local --no-mcp --no-learned-rules \
#     --tool fs_read --tool fs_list -w eval/workspace "<task>"
#
# then verifying it replays clean once, and appending its id to the file.
#
# Divergence semantics are `mecha replay`'s: argument-only differences are
# reported and fatal under --on-divergence=error; a structural divergence
# (different tool, different order) stops the replay. A pin that diverges
# means the harness — prompt assembly, tool dispatch, request shape — or the
# model changed; read the JSON before deciding which.
set -uo pipefail
cd "$(dirname "$0")/.."

PINS="${MECHA_REGRESSION_PINS:-$HOME/.mecha/regression-sessions.txt}"
MODEL_PORT=8080

if [ "$#" -gt 0 ]; then
  ids=("$@")
else
  if [ ! -f "$PINS" ]; then
    echo "no pin file at $PINS — record a session and list its id there" >&2
    exit 1
  fi
  mapfile -t ids < <(grep -vE '^\s*(#|$)' "$PINS")
  if [ "${#ids[@]}" -eq 0 ]; then
    echo "no pins listed in $PINS" >&2
    exit 1
  fi
fi

cargo build --release --quiet

# Seeded replay is only repeatable sequentially against one slot — the same
# -np 1 confound bench/run.sh guards against. Refuse rather than report
# fake divergence.
slots=$(curl -s "http://127.0.0.1:${MODEL_PORT}/props" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total_slots", 0))' 2>/dev/null)
if [ "${slots:-0}" != "1" ]; then
  echo "refusing to run: llama-server on :${MODEL_PORT} has ${slots:-no} slots, not 1 (-np 1)." >&2
  exit 1
fi

failed=0
for id in "${ids[@]}"; do
  if out=$(./target/release/mecha replay "$id" --on-divergence=error --json 2>&1); then
    echo "ok   $id"
  else
    failed=$((failed + 1))
    echo "FAIL $id"
    echo "$out" | tail -5 | sed 's/^/     /'
  fi
done

echo
echo "${#ids[@]} pin(s), ${failed} failed"
[ "$failed" -eq 0 ]
