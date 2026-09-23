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
# **Three skips are believed transient, and are only allowed to stay so.**
# A timeout, a 503 ("loading model") and a refused connection are what a
# busy or restarting server looks like — the port is closed for a moment
# while it restarts, then answers 503 while it loads — and also what a server
# that is gone, can never finish loading, or sits behind a black-holed port
# looks like forever. Persistence is what tells them
# apart, so consecutive skips of that kind are counted in a state file, and
# after MECHA_IDLE_STUCK_MAX of them in a row (default 9: three hours of
# ticks) the check fails instead. Any answer that proves the server alive —
# an idle or busy slot list — clears the count.
#
# **A busy slot is counted too, on a much longer fuse.** Busy is the normal
# reason to skip, and the owner chatting through an afternoon must never
# alarm; but a slot stuck `is_processing` — a wedged request, a run that never
# ends — would skip the sweep forever with nothing to say so. So consecutive
# busy skips get their own count, cleared by any tick that finds every slot
# idle, and fail after MECHA_IDLE_BUSY_MAX (default 36: twelve hours, longer
# than the whole daytime window).
set -uo pipefail

SLOTS_URL="${MECHA_SLOTS_URL:-http://127.0.0.1:8080/slots}"
GPU_BUSY="${MECHA_GPU_BUSY:-30}"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/mecha"
STUCK_MAX="${MECHA_IDLE_STUCK_MAX:-9}"
STUCK_FILE="$STATE_DIR/model-idle-stuck"
BUSY_MAX="${MECHA_IDLE_BUSY_MAX:-36}"
BUSY_FILE="$STATE_DIR/model-idle-busy"

# A skip that may be passing or may be permanent: counted in FILE, and loud
# once MAX of them have come in a row.
counted_skip() {
    local file=$1 max=$2 why=$3 n
    n="$(cat "$file" 2>/dev/null)"
    [ "$n" -eq "$n" ] 2>/dev/null || n=0
    n=$((n + 1))
    # A count that cannot be kept can never escalate, which would turn "loud
    # after three hours" into "quiet forever" — so that is a failure too.
    if ! { mkdir -p "$STATE_DIR" && printf '%s\n' "$n" >"$file"; } 2>/dev/null; then
        echo "model-idle: $why, and the skip count cannot be saved to $file — failing so mecha doctor sees it"
        exit 255
    fi
    if [ "$n" -ge "$max" ]; then
        echo "model-idle: $why for $n ticks in a row — failing so mecha doctor sees it"
        exit 255
    fi
    echo "model-idle: $why — skipping this run ($n in a row)"
    exit 1
}
stuck_skip() { counted_skip "$STUCK_FILE" "$STUCK_MAX" "$1"; }

# The body and the status separately: `curl -f` would fold every HTTP error
# into one exit code, and two of them mean opposite things here.
reply="$(curl -s -m 5 -w '\n%{http_code}' "$SLOTS_URL")"
rc=$?
code="${reply##*$'\n'}"
slots="${reply%$'\n'*}"
# "Not now" — skip, counted, and the next tick asks again:
#   curl 28, a timeout: something is listening but too busy to answer in 5 s
#     (a long prefill does that), which is the busiest the model gets;
#   curl 7, refused: nothing is bound to the port, which is what a restart
#     looks like before the server has opened it;
#   curl 52 or 56, an empty reply or a reset: a listening socket torn down
#     mid-restart, the moment before the refusal;
#   HTTP 503: llama-server is still loading the model after a restart.
# Failing on any of these would leave a false alarm in `mecha doctor` each
# time the server is bounced; the count makes a lasting one loud.
# Anything else — 501 (`--no-slots`), any other status, an answer it cannot
# read — is a server that is there and answering wrongly, and fails at once.
if [ "$rc" -eq 28 ]; then
    stuck_skip "$SLOTS_URL too busy to answer"
elif [ "$rc" -eq 7 ]; then
    stuck_skip "nothing listening at $SLOTS_URL"
elif [ "$rc" -eq 52 ] || [ "$rc" -eq 56 ]; then
    stuck_skip "$SLOTS_URL dropped the connection"
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
rm -f "$STUCK_FILE"
if [ "$busy" -gt 0 ]; then
    counted_skip "$BUSY_FILE" "$BUSY_MAX" "$busy model slot(s) in use"
fi
rm -f "$BUSY_FILE"

util="$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc '0-9')"
# Present but unreadable (`[N/A]`, which this box's nvidia-smi answers for
# its memory queries) still fails open, but says so: from the journal it must
# not look like a quiet GPU.
if [ -z "$util" ] && command -v nvidia-smi >/dev/null 2>&1; then
    echo "model-idle: nvidia-smi gave no readable GPU utilisation — not gating on it"
fi
if [ -n "$util" ] && [ "$util" -gt "$GPU_BUSY" ]; then
    echo "model-idle: GPU at ${util}% (> ${GPU_BUSY}%) — skipping this run"
    exit 1
fi

exit 0
