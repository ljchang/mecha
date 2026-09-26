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
# **Every skip is also counted for the day, in one count.** A busy slot and
# a busy GPU are the normal reasons to skip — the owner chatting, a research
# job on the card — and neither must alarm for an afternoon of it; but a slot
# stuck `is_processing` or a GPU pegged for days would skip the sweep forever
# with nothing to say so. So every skip, whatever its reason, adds to one
# count that belongs to the day (it starts over on the first tick of the
# next) and is cleared only when this check lets the sweep run; after
# MECHA_IDLE_DAY_MAX of them (default 44: every tick of the 07:30–21:50
# window) the check fails. One count, not one per reason: a day that
# alternates between a busy slot and a busy GPU never ran the sweep either,
# and per-reason counts that reset each other would never have said so.
# The alarm means one precise thing: the daytime sweep did not run once today.
set -uo pipefail

SLOTS_URL="${MECHA_SLOTS_URL:-http://127.0.0.1:8080/slots}"
GPU_BUSY="${MECHA_GPU_BUSY:-30}"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/mecha"
STUCK_MAX="${MECHA_IDLE_STUCK_MAX:-9}"
STUCK_FILE="$STATE_DIR/model-idle-stuck"
DAY_MAX="${MECHA_IDLE_DAY_MAX:-44}"
DAY_FILE="$STATE_DIR/model-idle-day"
# A placeholder if `date` fails, so the file keeps two fields and the count
# still climbs (it just never starts over for a new day).
TODAY="$(date +%F 2>/dev/null)"
TODAY="${TODAY:-unknown-day}"

# Add one to the count in FILE ("<date> <count>") and print it. With PERDAY
# set, a count from an earlier day starts over. Prints nothing if the count
# cannot be kept — the caller fails on that, because a count that cannot be
# kept can never escalate, turning "loud later" into "quiet forever".
bump() {
    local file=$1 perday=${2:-} day n
    read -r day n <"$file" 2>/dev/null || true
    [ "${n:-}" -eq "${n:-}" ] 2>/dev/null || n=0
    [ -n "$perday" ] && [ "${day:-}" != "$TODAY" ] && n=0
    n=$((n + 1))
    { mkdir -p "$STATE_DIR" && printf '%s %s\n' "$TODAY" "$n" >"$file"; } 2>/dev/null && echo "$n"
}

# Skip this tick for WHY: counted for the day, and — for a stuck server —
# in its own shorter count too. Loud once either has lasted too long.
skip() {
    local why=$1 stuck=${2:-} d s
    d="$(bump "$DAY_FILE" perday)"
    if [ -n "$stuck" ]; then s="$(bump "$STUCK_FILE")"; else s=0; fi
    if [ -z "$d" ] || [ -z "$s" ]; then
        echo "model-idle: $why, and the skip count cannot be saved under $STATE_DIR — failing so mecha doctor sees it"
        exit 255
    fi
    if [ -n "$stuck" ] && [ "$s" -ge "$STUCK_MAX" ]; then
        echo "model-idle: $why for $s ticks in a row — failing so mecha doctor sees it"
        exit 255
    fi
    if [ "$d" -ge "$DAY_MAX" ]; then
        echo "model-idle: $why — and the sweep has not run once today ($d ticks skipped) — failing so mecha doctor sees it"
        exit 255
    fi
    if [ -n "$stuck" ]; then
        echo "model-idle: $why — skipping this run ($s in a row, $d skipped today)"
    else
        echo "model-idle: $why — skipping this run ($d skipped today)"
    fi
    exit 1
}
stuck_skip() { skip "$1" stuck; }

# The GPU gate and the all-clear, shared by both ways of finding the slots
# idle below.
finish() {
    local util
    util="$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc '0-9')"
    # Present but unreadable (`[N/A]`, which this box's nvidia-smi answers for
    # its memory queries) still fails open, but says so: from the journal it
    # must not look like a quiet GPU.
    if [ -z "$util" ] && command -v nvidia-smi >/dev/null 2>&1; then
        echo "model-idle: nvidia-smi gave no readable GPU utilisation — not gating on it"
    fi
    if [ -n "$util" ] && [ "$util" -gt "$GPU_BUSY" ]; then
        skip "GPU at ${util}% (> ${GPU_BUSY}%)"
    fi
    # The sweep runs: the day's skips are over.
    rm -f "$DAY_FILE"
    exit 0
}

# **A router serves /slots per model** (llama-server router mode,
# REMOTE-SURFACE-DESIGN §14). Its bare /slots is a 400, and naming a model
# without `autoload=false` *loads* it — the idle check would be what swaps out
# the owner's pick. So ask which model is resident and read that one's slots.
# A server that does not say `role: router` (every single-model llama-server,
# and anything that does not answer /props) takes the plain read below
# unchanged, which fails loudly on its own terms.
BASE="${SLOTS_URL%/slots}"
role="$(curl -s -m 5 "$BASE/props" 2>/dev/null | python3 -c 'import json, sys; print(json.load(sys.stdin).get("role", ""))' 2>/dev/null)"
if [ "$role" = router ]; then
    # One line: "<status> <url-to-read>", "none", "many", or nothing on an
    # answer this cannot read.
    state="$(curl -s -m 5 "$BASE/models" 2>/dev/null | BASE="$BASE" python3 -c '
import json, os, sys, urllib.parse
r = [m for m in json.load(sys.stdin).get("data", [])
     if m.get("status", {}).get("value") in ("loaded", "loading", "sleeping")]
if not r:
    print("none")
elif len(r) > 1:
    print("many")
else:
    q = urllib.parse.urlencode({"model": r[0]["id"], "autoload": "false"})
    print(r[0]["status"]["value"], os.environ["BASE"] + "/slots?" + q)
' 2>/dev/null)"
    case "$state" in
        # Nothing loaded is nothing in flight: the sweep's first request loads
        # the default. A sleeping model has no request by definition.
        none | sleeping\ *)
            rm -f "$STUCK_FILE"
            finish
            ;;
        loading\ *) stuck_skip "the router at $BASE is loading a model" ;;
        loaded\ *) SLOTS_URL="${state#loaded }" ;;
        # Unreachable under `--models-max 1`, and loud on purpose: if the
        # router is ever run with room for two, this gate has to learn which
        # one the sweep will use before it can say "idle" — until then every
        # tick fails, which is the intended behaviour, not a bug.
        many)
            echo "model-idle: the router at $BASE has more than one model resident — not the one-model server this gate reads; failing so mecha doctor sees it"
            exit 255
            ;;
        *)
            echo "model-idle: the router at $BASE did not answer /models in a shape this reads — failing so mecha doctor sees it"
            exit 255
            ;;
    esac
fi

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
    skip "$busy model slot(s) in use"
fi

finish
