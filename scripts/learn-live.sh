#!/usr/bin/env bash
# Live learning: mine the session that just closed, and consolidate when
# enough has accumulated. Fired from a `session_end` hook.
#
# **The split this implements.** Mining and consolidation are cheap and want
# to be immediate — a lesson that lands tomorrow night missed everything the
# owner did today, which is flowmail's argument for firing compaction inline
# on a volume threshold rather than on a clock. *Measurement* is the opposite:
# a replay probe costs a real model run per arm per episode, so it stays in
# `ruminate.sh` where it can take its time and sit on a night's worth of
# evidence at once.
#
# Three things this must not do, each of which it would do naively:
#
# 1. **Block the session exiting.** `session_end` hooks are awaited, and a
#    consolidation is a model call. The hook launches this detached; nothing
#    downstream waits on it.
# 2. **Pile up.** Several sessions closing together would each start a pass,
#    and they would serialise on the learning store's own lock with the
#    losers achieving nothing. `flock -n` makes the second one exit instead:
#    whatever it would have mined is still unmined, so the next session end
#    (or the nightly) picks it up. Skipping is free; queueing is not.
# 3. **Report failure into a front-end that has already gone.** Hook failures
#    are logged, not surfaced, so everything here is best-effort by
#    construction and says so in the log rather than exiting non-zero.
set -uo pipefail

MECHA="${MECHA_BIN:-$HOME/.cargo/bin/mecha}"
PROVIDER="${MECHA_LEARN_PROVIDER:-local}"
LEARNING_DIR="${MECHA_LEARNING_DIR:-$HOME/.mecha/learning}"
LOG_DIR="$LEARNING_DIR/logs"
mkdir -p "$LOG_DIR"
LOG="$LOG_DIR/live-$(date -u +%Y-%m-%d).log"

# One live pass at a time. Held for the whole run, released on exit.
exec 9>"$LEARNING_DIR/.live.lock"
flock -n 9 || exit 0

{
  echo "── live learn $(date -u +%Y-%m-%dT%H:%M:%SZ) ──"
  # **Stand somewhere no repository's `mecha.toml` reaches.** A `session_end`
  # hook is spawned with `.current_dir(workspace)` — the closing session's
  # workspace — and `reflect`/`learn` both `Config::load(&cwd)`, which layers
  # `./mecha.toml` over the global config. `[providers]` is not stripped from
  # a project layer, so ending a session in an untrusted checkout would let
  # that repo point the consolidation call at its own endpoint, unattended,
  # with the reflections as payload — and whatever came back would be written
  # live into every future prompt's cached prefix. The provenance gate does
  # not help: it classifies the *reflections*, not the learner.
  #
  # `ruminate.sh` already carries this argument; moving consolidation to a
  # per-session hook is what dropped the protection.
  cd "$("$MECHA" work path learn-live)" || exit 0

  # A small limit: this fires per session, so there is normally one to mine.
  # The cap is what stops a hook that has not run for a week from turning one
  # session's exit into an hour of inference.
  "$MECHA" reflect -p "$PROVIDER" --limit 3 2>&1
  # Self-gating: `learn` refuses below --min and says so, so this is a no-op
  # on most sessions and a consolidation on the ones that tip it over.
  #
  # `--auto`, not bare `learn`: the candidate is replayed against the rules
  # currently deployed and refused if any probe comes out worse. A batch with
  # nothing gradeable in it still applies, marked probation and on a shorter
  # retirement leash — the D1 ruling, and the reason this is not simply
  # ungated.
  #
  # **`--holdout` even here.** `learn` marks everything it consumes processed,
  # and `validate --unprocessed-only` probes exactly what is left — so a hook
  # that fires per session would consume every reflection within minutes of
  # mining it and leave the nightly measurement with nothing. That ledger is
  # what `rules propose-retirements --apply` argues from and what the learner
  # reads as each rule's measured record, so starving it would render every
  # rule `[unmeasured]` forever and retire nothing. The holdout slice is
  # deterministic, so the slices accumulate across sessions into a real
  # measurement set.
  "$MECHA" learn -p "$PROVIDER" --holdout 0.25 --auto 2>&1
} >>"$LOG" 2>&1

exit 0
