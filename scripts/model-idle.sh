#!/usr/bin/env bash
# Exit 0 when the local model is free for background work, 1 when it is not.
#
# Written for systemd's `ExecCondition=`: an exit of 1–254 skips the unit's
# run *without* marking it failed, which is exactly "not now" — the next timer
# tick asks again. It prints why it declined, so a skipped run leaves a line
# in the journal instead of a gap.
#
# **What "free" means, and why it is two checks.** The daytime mail sweep
# must never be why the owner's chat is slow or why a research job loses the
# GPU, so it stands down if either is true:
#
#   - any slot on the local model server is processing. A slot in use is a
#     person or an agent run waiting on the model; a sweep beside it would
#     share the same decode and slow both. `/slots` answers per slot, so this
#     is exact rather than inferred from utilisation.
#   - GPU utilisation is above MECHA_GPU_BUSY (default 30%, the same line
#     mecha-graph's nightly uses). This is the research-work case: a training
#     or analysis job that is nothing to do with the model server.
#
# **A server that does not answer is a failure, not a skip** (exit 255, which
# `ExecCondition=` treats as the unit failing). Declining with 1 would be
# correct for this tick and invisible for good: a server restarted on another
# port, or a flag that stops serving `/slots`, would stand the sweep down
# every 20 minutes forever with only a journal line to say so. Failing puts
# it in front of `mecha doctor`, which watches `mecha-*` units. The nightly
# still catches up either way.
#
# **An answer that is not a readable slot count fails the same way**, for the
# same reason: a llama.cpp that renames `is_processing`, or a box that lost
# `python3`, would otherwise skip every tick forever and look exactly like a
# busy model. Unknown is never clean, and it is not quiet either. A missing
# or unreadable `nvidia-smi` does NOT decline: a box without a usable GPU
# query should still sort its mail, as the nightly's own check fails open.
set -uo pipefail

SLOTS_URL="${MECHA_SLOTS_URL:-http://127.0.0.1:8080/slots}"
GPU_BUSY="${MECHA_GPU_BUSY:-30}"

slots="$(curl -sf -m 5 "$SLOTS_URL")"
rc=$?
# curl 28 is a timeout: something is listening but too busy to answer in 5 s
# (a long prefill does that), which is the busiest the model gets — skip.
# Anything else (refused, reset, an HTTP error) is a server that is not
# there to ask, and fails loudly as above.
if [ "$rc" -eq 28 ]; then
    echo "model-idle: $SLOTS_URL too busy to answer — skipping this run"
    exit 1
elif [ "$rc" -ne 0 ]; then
    echo "model-idle: no answer from $SLOTS_URL (curl $rc) — failing so mecha doctor sees it"
    exit 255
fi
# A shape this does not recognise is not idle: an empty array, or slots
# whose busy flag is spelled some other way (it is llama.cpp's field, not a
# contract), must not sum to a confident zero. Exiting non-zero leaves
# `busy` empty, which the check below fails on.
busy="$(printf '%s' "$slots" | python3 -c '
import json, sys
s = json.load(sys.stdin)
if not isinstance(s, list) or not s or any("is_processing" not in x for x in s):
    raise SystemExit(2)
print(sum(1 for x in s if x["is_processing"]))
' 2>/dev/null)"
# The type, not a sentinel: python3 missing, killed or raising all leave
# `busy` empty or non-numeric, and every one of them fails loudly.
if ! [ "$busy" -eq "$busy" ] 2>/dev/null; then
    echo "model-idle: could not read a slot count from $SLOTS_URL — failing so mecha doctor sees it"
    exit 255
fi
if [ "$busy" -gt 0 ]; then
    echo "model-idle: $busy model slot(s) in use — skipping this run"
    exit 1
fi

util="$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc '0-9')"
if [ -n "$util" ] && [ "$util" -gt "$GPU_BUSY" ]; then
    echo "model-idle: GPU at ${util}% (> ${GPU_BUSY}%) — skipping this run"
    exit 1
fi

exit 0
