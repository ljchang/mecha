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
# An answer that is not a readable slot count also declines rather than
# guessing — unknown is never clean. A missing or unreadable `nvidia-smi`
# does NOT decline: a box without a usable GPU query should still sort its
# mail, as the nightly's own check fails open.
set -uo pipefail

SLOTS_URL="${MECHA_SLOTS_URL:-http://127.0.0.1:8080/slots}"
GPU_BUSY="${MECHA_GPU_BUSY:-30}"

slots="$(curl -sf -m 5 "$SLOTS_URL")" || {
    echo "model-idle: no answer from $SLOTS_URL — failing so mecha doctor sees it"
    exit 255
}
busy="$(printf '%s' "$slots" | python3 -c '
import json, sys
print(sum(1 for s in json.load(sys.stdin) if s.get("is_processing")))
' 2>/dev/null)"
# The type, not a sentinel: python3 missing, killed or raising all leave
# `busy` empty or non-numeric, and every one of them must decline.
if ! [ "$busy" -eq "$busy" ] 2>/dev/null; then
    echo "model-idle: could not read a slot count from $SLOTS_URL — skipping this run"
    exit 1
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
