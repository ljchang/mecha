#!/usr/bin/env bash
# Nightly rumination: mine → measure → learn.
#
# Ordering is the one deliberate choice here. `validate --unprocessed-only`
# runs BEFORE `learn`, because learn marks reflections processed — measuring
# afterwards would grade the rules on their own training data. Tonight's fresh
# reflections are unseen by the current rules by construction, and learn's
# --holdout keeps a slice unseen by the next generation too.
#
# Every stage is idempotent and defers on failure — the store lock serializes
# against any reflect a closing session fires, reflect leaves a session
# unmined if its provider call fails, and learn refuses to run under --min.
# So a dead model server just means tomorrow catches up, which is why the
# health check exits 0: a skipped night is not a failed night.
#
# Scheduling: scripts/mecha-ruminate.{service,timer} → ~/.config/systemd/user/
#   systemctl --user daemon-reload
#   systemctl --user enable --now mecha-ruminate.timer
#   loginctl enable-linger "$USER"     # timers must fire while logged out
set -uo pipefail

MECHA="${MECHA_BIN:-$HOME/.cargo/bin/mecha}"
PROVIDER="${MECHA_RUMINATE_PROVIDER:-local}"
JUDGE="${MECHA_RUMINATE_JUDGE:-gemma26}"
HEALTH="${MECHA_RUMINATE_HEALTH:-http://127.0.0.1:8080/health}"

LOG_DIR="${MECHA_LEARNING_DIR:-$HOME/.mecha/learning}/logs"
mkdir -p "$LOG_DIR"
exec >>"$LOG_DIR/$(date +%F).log" 2>&1

echo "── rumination start $(date -Is) ──"
if ! curl -sf -m 5 "$HEALTH" >/dev/null; then
    echo "model server not answering at $HEALTH; deferring the whole night"
    exit 0
fi

echo "· reflect"
"$MECHA" reflect -p "$PROVIDER"

echo "· validate (held-out + fresh, before learn consumes them)"
"$MECHA" validate -p "$PROVIDER" --judge-provider "$JUDGE" --unprocessed-only

echo "· learn (propose-only: unattended learning never applies its own output)"
"$MECHA" learn -p "$PROVIDER" --holdout 0.25 --propose

echo "· proposals awaiting review"
"$MECHA" proposals

echo "── rumination done $(date -Is) ──"
