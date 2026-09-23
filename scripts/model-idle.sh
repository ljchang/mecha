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
#
# **Two skips are believed transient, and are only allowed to stay so.** A
# timeout and a 503 ("loading model") are what a busy or restarting server
# looks like — and also what a server that can never finish loading, or a
# black-holed port, looks like forever. Persistence is what tells them
# apart, so consecutive skips of that kind are counted in a state file, and
# after MECHA_IDLE_STUCK_MAX of them in a row (default 9: three hours of
# ticks) the check fails instead. Any answer that proves the server alive —
# an idle or busy slot list — clears the count, as does a GPU skip, which
# says nothing about the server.
set -uo pipefail

SLOTS_URL="${MECHA_SLOTS_URL:-http://127.0.0.1:8080/slots}"
GPU_BUSY="${MECHA_GPU_BUSY:-30}"
STUCK_MAX="${MECHA_IDLE_STUCK_MAX:-9}"
STUCK_FILE="${XDG_STATE_HOME:-$HOME/.local/state}/mecha/model-idle-stuck"

# A transient-looking skip: counted, and loud once it has lasted too long.
stuck_skip() {
    local n
    n="$(cat "$STUCK_FILE" 2>/dev/null)"
    [ "$n" -eq "$n" ] 2>/dev/null || n=0
    n=$((n + 1))
    mkdir -p "$(dirname "$STUCK_FILE")" && printf '%s\n' "$n" >"$STUCK_FILE"
    if [ "$n" -ge "$STUCK_MAX" ]; then
        echo "model-idle: $1 for $n ticks in a row — failing so mecha doctor sees it"
        exit 255
    fi
    echo "model-idle: $1 — skipping this run ($n in a row)"
    exit 1
}
clear_stuck() { rm -f "$STUCK_FILE"; }

# The body and the status separately: `curl -f` would fold every HTTP error
# into one exit code, and two of them mean opposite things here.
reply="$(curl -s -m 5 -w '\n%{http_code}' "$SLOTS_URL")"
rc=$?
code="${reply##*$'\n'}"
slots="${reply%$'\n'*}"
# "Not now" — skip, and the next tick asks again:
#   curl 28, a timeout: something is listening but too busy to answer in 5 s
#     (a long prefill does that), which is the busiest the model gets;
#   HTTP 503: llama-server is still loading the model, which it does after
#     every restart — failing here would leave a false alarm in `mecha doctor`
#     each time the server is bounced.
# Anything else — refused, reset, 501 (`--no-slots`), any other status — is a
# server that is not there to ask, and fails loudly as above.
if [ "$rc" -eq 28 ]; then
    stuck_skip "$SLOTS_URL too busy to answer"
elif [ "$rc" -ne 0 ]; then
    echo "model-idle: no answer from $SLOTS_URL (curl $rc) — failing so mecha doctor sees it"
    exit 255
elif [ "$code" = 503 ]; then
    stuck_skip "$SLOTS_URL says the model is still loading"
elif [ "$code" != 200 ]; then
    echo "model-idle: $SLOTS_URL answered HTTP $code — failing so mecha doctor sees it"
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
# A readable slot list: the server is alive, whatever it is doing.
clear_stuck
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
