#!/usr/bin/env bash
# The front door, unattended: drain → extract → triage.
#
# Ordering is the design. `drain` runs first and unconditionally, because it
# is the zero-token leg — an HTTP fetch that needs no model, so a dead model
# server must not stop requests coming home. The model health check sits
# *after* it, guarding only the two stages that spend tokens. `extract` runs
# before `triage` because triage drafts only from extractions — the privileged
# run sees the typed fields, never the prose. `reconcile` is deliberately
# absent: it runs on its own inside the frontdoor verbs, because a state that
# is only correct after someone runs a command is a state nobody can trust.
#
# Every stage defers on failure rather than failing the unit: a skipped hour
# is not a failed hour, and the next tick catches up. Triage refuses to run
# without the outbox route, which is the correct failure — a draft that could
# actually send is worse than no draft.
#
# Scheduling: scripts/mecha-frontdoor.{service,timer} → ~/.config/systemd/user/
#   systemctl --user daemon-reload
#   systemctl --user enable --now mecha-frontdoor.timer
#   loginctl enable-linger "$USER"     # timers must fire while logged out
set -uo pipefail

MECHA="${MECHA_BIN:-$HOME/.cargo/bin/mecha}"
FACTORY="${FACTORY_PUBLISH_BIN:-$HOME/.cargo/bin/factory-publish}"
PROVIDER="${MECHA_FRONTDOOR_PROVIDER:-local}"
HEALTH="${MECHA_FRONTDOOR_HEALTH:-http://127.0.0.1:8080/health}"

LOG_DIR="$HOME/.mecha/requests/logs"
mkdir -p "$LOG_DIR"
exec >>"$LOG_DIR/$(date +%F).log" 2>&1

echo "── frontdoor tick $(date -Is) ──"

# Stand somewhere the path jail accepts before any stage that builds a tool
# surface. Triage builds one (its drafts stage mail tools through the outbox),
# and the workspace check refuses any directory *containing* `~/.mecha` — which
# is where a user unit with no WorkingDirectory stands. Same reasoning, at
# length, in ruminate.sh.
if ! cd "$("$MECHA" work path frontdoor)"; then
    echo "cannot resolve the frontdoor work directory; deferring this tick"
    exit 0
fi

echo "· drain (zero tokens; the common case is nothing new)"
"$FACTORY" drain || echo "drain failed; extracting and triaging what is already home"

if ! curl -sf -m 5 "$HEALTH" >/dev/null; then
    echo "model server not answering at $HEALTH; drained only, deferring the rest"
    exit 0
fi

echo "· extract (the quarantined pass: no tools, no history)"
"$MECHA" frontdoor extract -p "$PROVIDER"

echo "· triage (drafts into the outbox; refuses to run unrouted)"
"$MECHA" frontdoor triage -p "$PROVIDER" --read-only

echo "── frontdoor tick done $(date -Is) ──"
