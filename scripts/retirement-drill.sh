#!/usr/bin/env bash
# The retirement end-to-end drill: prove the learning loop's NoGo path fires
# as one motion, with a real model in the loop.
#
#   seeded bad rule → mecha validate probes a steer → the rules-on arm
#   regresses → the bisection names the bad rule → the ledger accumulates
#   attributed regressions → mecha rules propose-retirements --apply
#   convicts it at the probation leash (2), while an innocuous bystander
#   rule in the same block survives.
#
# Why this exists: LEARNING-LOOP-RESEARCH.md made it a precondition of
# ungating — "a NoGo pathway that has never fired is an untested backstop,
# and under D1 it is the only one." Every piece is unit-tested; this is the
# joined motion. Running it found the first bug the same day it was written:
# the probation release keyed on coverage, which conviction evidence always
# supplies, so the 2-threshold could never convict anything.
#
# The drill is fully isolated: MECHA_SESSION_DIR and MECHA_LEARNING_DIR
# point every stage at a throwaway world, so the live learning store never
# sees a fabricated ledger row. The surface store (~/.mecha/surface) is
# shared on purpose — it is content-addressed, and the recording's blob is
# real. The scenario is seeded (a steer inserted into an honest recording,
# by mecha-core's own types, never jq), because a *seeded regression* is
# what the drill was specified to convict on; the model runs are real, so
# the verdicts are the model's own and a drift in its behaviour surfaces
# here as a diagnosed failure, not a silent pass.
#
#   scripts/retirement-drill.sh            # run the whole drill
#   MECHA_DRILL_KEEP=1 …                   # keep the world for a post-mortem
set -uo pipefail
cd "$(dirname "$0")/.."

MODEL_PORT="${MECHA_DRILL_MODEL_PORT:-8080}"
if ! curl -sf -m 5 "http://127.0.0.1:${MODEL_PORT}/health" >/dev/null; then
    echo "model server not answering on :${MODEL_PORT}; the drill needs a real model" >&2
    exit 1
fi

echo "── retirement drill: building ──"
cargo build --release -p mecha-cli --quiet || exit 1
cargo build --release --example retirement_drill_seed -p mecha-core --quiet || exit 1
# Absolute, because every mecha stage runs from inside the drill workspace:
# config is layered from the cwd, and an unattended drill must not pick up a
# project mecha.toml from whatever checkout it was launched in — the same
# argument the triggers unit makes. (A cwd under ~/.mecha would also be
# refused by the workspace check; a mktemp world is safely outside it.)
MECHA="$(pwd)/target/release/mecha"
SEED="$(pwd)/target/release/examples/retirement_drill_seed"

DRILL="$(mktemp -d /tmp/mecha-retirement-drill.XXXXXX)"
export MECHA_SESSION_DIR="$DRILL/sessions"
export MECHA_LEARNING_DIR="$DRILL/learning"
mkdir -p "$DRILL/ws"
cleanup() {
    if [ -n "${MECHA_DRILL_KEEP:-}" ]; then
        echo "drill world kept at $DRILL"
    else
        rm -rf "$DRILL"
    fi
}
trap cleanup EXIT

fail() {
    echo "DRILL FAILED: $1" >&2
    echo "── diagnostics ──" >&2
    echo "· ledger ($MECHA_LEARNING_DIR/validations.jsonl):" >&2
    cat "$MECHA_LEARNING_DIR/validations.jsonl" 2>/dev/null >&2 || echo "  (none)" >&2
    echo "· rules:" >&2
    "$MECHA" rules 2>&1 | sed 's/^/  /' >&2
    MECHA_DRILL_KEEP=1
    exit 1
}

# ── 1. record an honest session ──────────────────────────────────────────
# A real run against the real model: read one file, answer. fs_list rides on
# the recorded surface unused, so the bad rule's push toward it is
# expressible in the replay. Hooks, MCP, skills and learned rules are all
# off: the recording should carry nothing but the scenario.
echo "── 1. recording a session (real run, real model) ──"
echo "The standup moved to 09:30 on Thursdays." > "$DRILL/ws/notes.txt"
(cd "$DRILL/ws" && "$MECHA" run -p local --no-mcp --no-hooks --no-skills --no-learned-rules \
    --tool fs_read --tool fs_list -w "$DRILL/ws" \
    "Read notes.txt and tell me what it says." >/dev/null) \
    || fail "the recording run did not complete"

SESSION="$(basename "$(ls -t "$MECHA_SESSION_DIR"/*.jsonl | head -1)" .jsonl)"
[ -n "$SESSION" ] || fail "no session was recorded"
echo "recorded session $SESSION"

# ── 2. seed the scenario ─────────────────────────────────────────────────
# The steer agrees with what the recording already did, so the rules-free
# arm tracks it and passes; the bad rule (probationary) demands a call the
# recording never makes, which is the regression; the bystander is the
# bisection's foil.
echo "── 2. seeding: steer + reflection + bad rule (probation) + bystander ──"
STEER="Just answer from what you already read — no more tool calls."
"$SEED" "$MECHA_SESSION_DIR" "$SESSION" "$MECHA_LEARNING_DIR" "$STEER" \
    || fail "seeding did not complete"

# ── helpers over the isolated stores ─────────────────────────────────────
rule_state() { # rule_state <id> <field>
    "$MECHA" rules list --json | python3 -c "
import json, sys
rows = json.load(sys.stdin)
row = next(r for r in rows if r['id'] == '$1')
v = row['$2']
print(json.dumps(v) if not isinstance(v, str) else v)"
}

ledger_attributed() { # attributed-regression rows charged to the bad rule
    python3 -c "
import json
n = 0
try:
    for line in open('$MECHA_LEARNING_DIR/validations.jsonl'):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get('outcome') == 'regressed' and r.get('attributed_rule_id') == 'r-drill-bad':
            n += 1
except FileNotFoundError:
    pass
print(n)"
}

# ── 3. two probe passes, a retirement attempt after each ─────────────────
# Each validate pass drives the steer probe once per arm against the real
# model and appends one ledger row; a regression bisects before it lands.
# After one conviction the scan must hold (the leash is 2, not a hair
# trigger); after two it must convict the bad rule at the probation
# threshold and leave the bystander alone.
for PASS in 1 2; do
    echo "── 3.$PASS. validate (probe pass $PASS) ──"
    (cd "$DRILL/ws" && "$MECHA" validate -p local --no-mcp --no-hooks --no-skills --trigger steer) \
        || fail "validate pass $PASS did not complete"
    ATTR="$(ledger_attributed)"
    echo "ledger: $ATTR attributed regression(s) against r-drill-bad"
    [ "$ATTR" -eq "$PASS" ] || fail \
        "pass $PASS should leave $PASS attributed regression(s), found $ATTR — \
the probe did not regress on the bad rule (model behaviour drifted? read the ledger above)"

    echo "── retirement scan after pass $PASS ──"
    "$MECHA" rules propose-retirements --apply || fail "the retirement scan errored"
    ACTIVE="$(rule_state r-drill-bad active)"
    if [ "$PASS" -eq 1 ]; then
        [ "$ACTIVE" = "true" ] || fail "one conviction retired the rule — the leash is 2, not 1"
        echo "held: one conviction is one bisection's opinion; the rule stays"
    else
        [ "$ACTIVE" = "false" ] || fail \
            "two attributed regressions did not retire the probationary rule — the D1 leash did not fire"
        REASON="$(rule_state r-drill-bad retired_reason)"
        case "$REASON" in
        *probation*) echo "convicted at the probation leash: $REASON" ;;
        *) fail "retired, but not at the probation leash: $REASON" ;;
        esac
    fi
    [ "$(rule_state r-drill-bystander active)" = "true" ] \
        || fail "the bystander rule was convicted — the bisection charged the wrong rule"
done

echo
echo "── DRILL PASSED ──"
echo "seeded bad rule → regressed probe → bisection → propose-retirements"
echo "fired as one motion: convicted at the probation leash (2) on the exact"
echo "rule at fault, bystander untouched, everything in an isolated world."
