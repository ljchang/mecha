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

# Stand somewhere the path jail accepts, before any stage that builds a tool
# surface. `validate` and `learn --propose` both replay against the live
# registry, so both go through the workspace check that refuses any directory
# *containing* `~/.mecha` — and a systemd user unit with no WorkingDirectory
# runs in `$HOME`, which contains it.
#
# What makes that worth fixing rather than noting: those two build the surface
# lazily, only when there is a proposal to gate or a steer/denial probe to
# replay. So the failure skips the nights that have work and passes the quiet
# ones, and this script is deliberately not `set -e`, so it would skip them
# without stopping. A nightly job that works until the night it matters is the
# shape of bug this project keeps finding.
#
# `mecha work path` builds no surface of its own, so it cannot be caught by the
# check it is resolving, and it creates the directory on the way past. Done
# here rather than with `WorkingDirectory=` in the unit so the script is
# correct however it is launched — by hand, by cron, or by the timer. A
# checkout would be the wrong answer either way: it would put a project's
# `mecha.toml` in front of an unattended run, which is exactly what the
# triggers unit refuses to do for the same reason.
if ! cd "$("$MECHA" work path ruminate)"; then
    echo "cannot resolve the ruminate work directory; deferring the whole night"
    exit 0
fi

if ! curl -sf -m 5 "$HEALTH" >/dev/null; then
    echo "model server not answering at $HEALTH; deferring the whole night"
    exit 0
fi

echo "· reflect"
"$MECHA" reflect -p "$PROVIDER"

echo "· distill (episodes → the knowledge graph; catches whatever a hook missed)"
"$MECHA" distill -p "$PROVIDER"

echo "· validate (held-out + fresh, before learn consumes them)"
"$MECHA" validate -p "$PROVIDER" --judge-provider "$JUDGE" --unprocessed-only

echo "· learn (propose-only: unattended learning never applies its own output)"
"$MECHA" learn -p "$PROVIDER" --holdout 0.25 --propose

echo "· retirements (deterministic ledger scan; staged for review like any rule change)"
"$MECHA" rules propose-retirements

echo "· work clean (retention on generated output; a published bundle's source is never removed)"
"$MECHA" work clean

echo "· proposals awaiting review"
"$MECHA" proposals

echo "── rumination done $(date -Is) ──"
