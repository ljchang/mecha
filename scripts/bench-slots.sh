#!/usr/bin/env bash
# Measure what a slot configuration actually costs, against the running
# llama-server on :8080.
#
# Why this exists: `start-moe-mtp.sh` carries a measured table with entries for
# `-np 1` and `-np 4` and nothing between, and its own standing rule is to
# **measure tokens/sec after restarting, not just that the server came back
# up** — because a server that loads while memory is contended stays slow for
# its whole life, and nothing logs that. Re-deriving the numbers by hand each
# time is how the table went three days carrying a conclusion drawn from one
# bad afternoon.
#
# Two arms, matching the table's units:
#   single   one stream, 300 generated tokens, median of 3   -> interactive speed
#   fanout   N concurrent streams, 300 each                  -> aggregate throughput
#
# Uses /completion rather than /v1/chat/completions on purpose: `ignore_eos`
# makes every run generate exactly n_predict tokens, so the arms are comparable
# token-for-token, and the server reports its OWN timings, so nothing here
# measures curl.
#
# Usage:  scripts/bench-slots.sh [label] [fanout_n]
set -uo pipefail

HOST="${LLAMA_HOST:-http://127.0.0.1:8080}"
LABEL="${1:-unlabelled}"
FANOUT="${2:-4}"
NPREDICT="${NPREDICT:-300}"

if ! curl -sf -m 5 "$HOST/health" >/dev/null 2>&1; then
    echo "bench: no server answering at $HOST/health" >&2
    exit 1
fi

slots_configured() {
    curl -s "$HOST/props" 2>/dev/null \
      | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total_slots","?"))' 2>/dev/null
}

# One request. Prints the server's reported generation rate.
# $1 = a prompt seed, so concurrent streams do not collide on one slot's prefix
one() {
    curl -s "$HOST/completion" -H 'Content-Type: application/json' -d "{
        \"prompt\": \"[$1] Write a detailed technical description of a distributed system.\",
        \"n_predict\": $NPREDICT,
        \"ignore_eos\": true,
        \"cache_prompt\": false,
        \"temperature\": 0.8,
        \"seed\": 42
    }" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("nan"); sys.exit()
rate = d.get("timings", {}).get("predicted_per_second")
print("nan" if rate is None else "%.2f" % rate)
'
}

echo "=== $LABEL ==="
echo "server reports total_slots = $(slots_configured)"

# --- arm 1: single stream, median of 3 -------------------------------------
vals=()
for i in 1 2 3; do
    v=$(one "single$i")
    vals+=("$v")
    printf '  single run %d: %s tok/s\n' "$i" "$v"
done
MEDIAN=$(printf '%s\n' "${vals[@]}" | sort -n | sed -n 2p)
echo "  single-stream MEDIAN: $MEDIAN tok/s"

# --- arm 2: N concurrent ----------------------------------------------------
# Distinct prompts, so each stream wants its own slot rather than matching an
# existing one by longest-common-prefix.
TMP=$(mktemp -d)
start=$(date +%s.%N)
for i in $(seq 1 "$FANOUT"); do
    one "fanout-stream-$i-distinct-prefix" >"$TMP/$i" &
done
wait
end=$(date +%s.%N)
WALL=$(echo "$end - $start" | bc)

each=$(cat "$TMP"/* | tr '\n' ' ')
# Sum of the server's per-request rates. Keep it for diagnosis, but do NOT
# read it as throughput: llama-server times a request only while it is
# *running*, so queue wait is invisible to it. At `-np 1` four serialized
# streams each report their full solo rate and the sum reads 4x — a number
# that would say slots are unnecessary, measured on the one configuration
# that cannot provide them. Wall clock is the only honest aggregate.
SUMRATES=$(cat "$TMP"/* | python3 -c '
import sys
v=[float(x) for x in sys.stdin if x.strip() and x.strip()!="nan"]
print("%.2f" % sum(v) if v else "nan")
')
rm -rf "$TMP"

TOTAL=$((FANOUT * NPREDICT))
AGG=$(python3 -c "print('%.2f' % ($TOTAL / $WALL))")

echo "  ${FANOUT}-stream per-stream (in-flight rates): $each"
echo "  ${FANOUT}-stream sum-of-rates:  $SUMRATES tok/s   <- excludes queue wait, not throughput"
echo "  ${FANOUT}-stream THROUGHPUT:    $AGG tok/s   ($TOTAL tokens in ${WALL}s wall)"
echo
echo "summary,$LABEL,single_median=$MEDIAN,fanout${FANOUT}_throughput=$AGG"
