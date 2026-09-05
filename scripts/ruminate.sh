#!/usr/bin/env bash
# Nightly rumination: measure → sweep → retire.
#
# **Mining and consolidation moved live** (`learn-live.sh`, a `session_end`
# hook): a lesson that lands tomorrow night missed everything the owner did
# today. What stays nightly is the expensive half — the replay probes behind
# `validate`, which cost a real model run per arm per episode and are better
# spent once on a night's evidence than per session. `reflect` and `learn`
# remain here as a *sweep*, for sessions whose hook did not fire (a crashed
# front-end, a machine that was asleep) — not as the primary path.
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
# **The judge is the model under test, deliberately and provisionally.**
# A different family is the better methodology — a model grading trajectories
# it produced shares the blind spot that caused them, which is why
# `validate --judge-provider` exists at all. But gemma26 is served on :8082
# and nothing has been listening there, so the independent judge was grading
# *nothing*: 22 of 39 reflections skipped on transport failures. An available
# correlated judge beats an unavailable independent one, and this is Luke's
# call (2026-08-29) rather than a discovery.
#
# Set MECHA_RUMINATE_JUDGE=gemma26 after starting scripts/start-gemma26.sh to
# put the independence back. Judge-graded rows in the ledger are only as good
# as this line.
JUDGE="${MECHA_RUMINATE_JUDGE:-local}"
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
# correct however it is launched — by hand, by cron, or by the timer.
#
# **Standing here is still right, and it is no longer the same thing as being
# unable to read the source.** This comment used to end "a checkout would be
# the wrong answer either way: it would put a project's `mecha.toml` in front
# of an unattended run" — true of the *config* root and not of the *jail* root,
# and conflating the two left the diagnostician jailed in an empty directory
# while its prompt told it to read the documentation. Config is discovered from
# the cwd and the jail is rooted at the workspace, so the fix was to keep
# standing outside a checkout and point at one: set `[harness] source_dir` in
# `~/.mecha/config.toml` and `mecha harness ruminate` reads the source
# read-only with `global_config_only` pinned, so no project layer rides in.
# Unset, it says it is blind rather than pretending otherwise. See
# `docs/SELF-IMPROVEMENT-RESEARCH.md` §14.1.
if ! cd "$("$MECHA" work path ruminate)"; then
    echo "cannot resolve the ruminate work directory; deferring the whole night"
    exit 0
fi

if ! curl -sf -m 5 "$HEALTH" >/dev/null; then
    echo "model server not answering at $HEALTH; deferring the whole night"
    exit 0
fi

echo "· reflect (catches whatever the session_end hook missed; live mining is learn-live.sh)"
"$MECHA" reflect -p "$PROVIDER"

echo "· distill (episodes → the knowledge graph; catches whatever a hook missed)"
"$MECHA" distill -p "$PROVIDER"

echo "· validate (the measurement: held-out + fresh, before learn consumes them;"
echo "  --cover 1 buys one probe per (rule, region) pair the ledger has never graded,"
echo "  so a widened rule is measured in each sub-region it widened over)"
"$MECHA" validate -p "$PROVIDER" --judge-provider "$JUDGE" --unprocessed-only --cover 1

echo "· learn (sweep: live consolidation runs per session, this catches the remainder;"
echo "  --auto measures the candidate and applies it, or refuses it, without staging)"
"$MECHA" learn -p "$PROVIDER" --holdout 0.25 --auto

echo "· retirements (deterministic ledger scan; applied, not staged — a rule measured"
echo "  harmful must leave the prompt without waiting for anyone, and it is the only"
echo "  brake on rules that now go live when they are derived)"
"$MECHA" rules propose-retirements --apply

echo "· work clean (retention on generated output; a published bundle's source is never removed)"
"$MECHA" work clean

echo "· harness (diagnose one change from the run corpus, measure it by counterfactual"
echo "  replay of recent sessions, and dispose through the candidate gate — a measured,"
echo "  holdout-confirmed config win auto-applies to the override layer, reversibly;"
echo "  prose, architecture and anything unmeasurable stages for review)"
"$MECHA" harness ruminate -p "$PROVIDER" --sessions 16

echo "· proposals awaiting review"
"$MECHA" proposals

echo "· harness candidates awaiting review"
"$MECHA" harness list

echo "── rumination done $(date -Is) ──"
