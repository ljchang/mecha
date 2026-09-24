# The appraisal system as built — inventory, readers and the measured record

Researched 2026-09-24 against `main` at `b6cfa73c`, refreshed at `9eea04e7`.
One question: **what appraisal-related functionality exists across mecha and
its sibling repositories, what reads each signal, and what has been
measured?** The answer is the evidence base for
[`APPRAISAL-WIRING-DESIGN.md`](APPRAISAL-WIRING-DESIGN.md), which decides
what to build from it. The signals themselves are designed in
`GOAL-SYSTEM-DESIGN.md` and their invariants are ARCHITECTURE's goal-system
section; neither is restated.

Method: ten passes. Five on the first day covered a signal inventory, a map
of harness decision points, the live corpus (aggregates only,
`MECHA_SESSION_KIND=test` on every readout) and two literature reviews. Five
more covered the core goal/assistant modules, learning and self-improvement,
every owner-facing surface, the sibling repositories, and the record (HISTORY,
HANDOFF, the design docs and ~45 merged PRs). Every shipped claim was
sample-checked for a symbol on `main`. Live-store figures are dated; they
describe this install on that day.

A bare §N is `GOAL-SYSTEM-DESIGN.md`'s. Proposal ids (S1, L7, …) are the
wiring design's.

**Reader classes** used throughout: **A** changes the model's in-run
behaviour, **B** changes a harness decision, **C** owner-facing display,
**D** recorded and unread.

---

## 1. What reads each signal today

| signal | reader today | class | live on this install |
|---|---|---|---|
| Boredom notice (`boredom::Boredom::observe_turn`) | templated line into the transcript | A | yes — 2 runs in 30 days |
| Predicted context size (`pressure::ContextTracker`) | early compaction, `Agent::output_budget` | B | yes — the only live disposition |
| `Situation` (`situation::Situation::of_run`) | `LearningStore::rules_carried_for` scopes which rules load | B | yes — the only live relevance mechanism |
| `Appraisal::cut_short`, `Affect::names_residue` | `tasks set --status done` stages a follow-up | B | yes, owner-triggered |
| `appraisal::charter_rank` | `harness_probe::selection_order` tiebreak after `Metric::headroom` | B | tiebreak only |
| `OutboxItem::predictions` | `ensure_prediction_ready` blocks release | B | only after `mecha outbox anticipate` per draft |
| `planning::Decision` | `Action::guidance` into the `todo` result | A | **no** — `goal_guidance` off, and no plans |
| Frozen step check (`step::CHECK_TRACE`) | signed −1.0, `extract_mismatches` | A+B | **no** — no check ever declared |
| `goal_context` (`learning::goal_lessons`, `planning::examples`) | registered tool | A | **no** — 0 of 66 reflections carry a goal |
| Goal anchor (`Conversation::goal_anchor`) | `Decision`, drift, `goal_context` | A | **no** — 123 anchor records, all null |
| `anticipated_guilt`, `peak_context_pressure` | `diagnose::Evidence` means | weak B | recorded; guilt reads 0.95–1.0 in every run |
| Charter readings (`Homeostat::charter`) | `Decision::assess`, doctor saturation, `/charter` | gated A / C | the one sensor reads over-setpoint in 126 of 126 runs |
| Valence, `Affect` | TUI badge, web chip, voice `cfg_weight`, Slack line, `distill` meta | C | yes |
| `distill::Surprise` | printed; a person may run `gossip` | C | yes |
| `peak_prompt_tokens`, `guilt_after_relief`, `mem_available_kb` | none | D | — |

Corpus (30 days, 245 real runs, test and unmarked development runs excluded):
web 171, trigger 29, delegated task 25, voice 18, front door 2. Median two tool
calls; 34% make none; 79 make more than three. **86% of real runs carry both
private and untrusted taint** — nearly every run is read under an armed
interlock. `todo` 5 runs, `ask_user` 5, `recall` 0, `goal_context` 0,
`subagent` 0. No compaction, no context overflow, no `MaxTurns`. The goal
sentence the charter block asks for has been in 68 sessions since 2026-09-06
and appears in none of them. The learning store is 100% corrections: 66
reflections (follow-up 36, steer 22, denial 4, edit 4), 4 active rules.

Doc/code disagreements found on the way. Each was corrected on
`docs/appraisal-wiring` the same day (the code comment in `distill.rs`,
ARCHITECTURE's goal-system section, and the website's appraisal pages):
- `distill.rs` (and the website's distillation page) say `meta.affect` and
  the goal errors give the graph's review queue a salience order. mecha-graph
  has no reader of either; GOAL-SYSTEM-DESIGN's rung 9 row calls it "not
  verified", and it is verified unbuilt.
- §10.1's "high-surprise sessions seed gossip": gossip *is* seeded every
  night, by mecha-graph's Selector (demand × gap × staleness, through
  `nightly-mecha.sh`) — never by `distill::Surprise`.
- ARCHITECTURE's "two sensors whose only reader is the diagnostician's
  brief" undercounts: the charter reading also reaches `Decision::assess`
  and the doctor's saturation check, and the brief itself can be switched
  off (`sensors_in_brief`).
- ARCHITECTURE's commitment paragraph still lists a negative
  `backlog_delta` as a +0.5 `Own` error. `of_session` no longer produces it;
  the earlier draft of this file repeated the stale claim.
- `Channel::Setpoint` and `GoalRef::Setpoint` have no production producer —
  they exist because the enums are a wire format — and the website's
  reference page presents `setpoint` as a live channel.

---

## 2. Two systems already built, and idle

The table in §1 is per signal. Two of those signals are halves of larger
systems that were designed and largely built. Both are starved by the same
missing input: a goal on the run.

**Goal inference and alignment tracking** (§17.3, §17.7 items 3–5; the
owner's *Latent Goal Inference, Goal Error Tracking, and Episodic Learning*
proposal of 2026-09-03 is absorbed there). A pipeline in four stages:

| stage | built | what fills it today |
|---|---|---|
| **Hypothesis** — "I take the goal to be X, serving Y" | the charter block asks for the sentence; `ask_user` takes `goal` and `serves` and carries a typed `GoalHypothesis`; the delegated seed folds it into its one question; an unattended run's goal is derived at review time by `outbox_source::serves_at_staging` | the model, and it never has: 0 of 68 sessions |
| **Confirmation** — the one label the agent did not author | a present human answering `ask_user` (`Reply::Answered`); a parked `Question` answered; the owner releasing a draft whose note names the goal; `run --goal` | 0 goals put to the owner, 0 answered |
| **Anchor** — the confirmed pointer | `GoalTrack` per run; `Conversation::goal_anchor` across turns, `Record::GoalAnchor` through resume and compaction; `questions::seed_anchor` on resume | 123 anchor records, all null |
| **Alignment** — does the work still trace to it | `goal::drift_of` per plan write — same, *changed pointer*, or *unnamed*; `goal_drift_rate` in `sessions health`; `Decision::ClarifyGoal` when plan and anchor differ | no plan writes, so no reading |

Named and unbuilt in §17.7 item 4: the re-ask on a changed pointer, the drift
*event*, the turns-since-confirmation term, and §17.3's "distance × remaining
work exceeds the cost of asking" check-in — all deliberately "off until the
rate is read", and the rate cannot be read while nothing fills stage one.
Also unbuilt: any *semantic* reading of the owner's answer — the anchor is a
pointer, so a correction in the answer's prose moves nothing.

**The validator stack.** The tree validates at four tiers and grounds claims
at a fifth; §17.5 names the tier it lacks.

| validates | against | how | state |
|---|---|---|---|
| a completed **step** (`step::appraise`) | the plan | deterministic reading of the step's span (null, failed last call, verify-less claim); a quarantined second opinion only on ambiguity (`escalate_step`) | built; plan-gated, so idle; escalation off by default |
| a declared **check** (`step::CheckRequest`, `CHECK_TRACE`) | the model's own frozen predicate | executed through ordinary dispatch; can manufacture a failed check against itself, never a passing one | built; reachable only from `todo`, so idle |
| a learned **rule** (`mecha validate`, `counterfactual.rs`) | the owner's recorded intervention | branch the transcript at the intervention, strip the steer, see whether the model now does the steered thing; per-region since #192; probation and retirement from the ledger | **live** — 334 ledger rows since 2026-08-29, the latest today (none 09-18 to 09-23): 18 improved, 17 regressed, 37 unchanged-pass, 139 unchanged-fail, 123 inconclusive |
| a task **artifact** (`mismatch::ArtifactCase`) | owner-supplied gold, outside the workspace | criterion-by-criterion, after the run; feeds `extract_mismatches` | built; used by experiments, no live producer |
| a **claim** (`grounding::admit`, `calls`) | what the run actually received | dereference; first seen wins; stale is never evidence | built for front-door dates and triage deadlines |
| a **graph claim** (`gossip`: `vet_judge`, `corroboration_verdict`, `round_yield`) | an independent reader over separate sources | two lensed readers, commit then reveal | live nightly, seeded by the graph's Selector |
| a **delivery** (`OutboxItem::delivery_uncertain`, `ensure_delivery_ready`, `outbox reconcile`) | the provider's record of what arrived | blocks release while delivery is uncertain | live gate; the precondition for any post-delivery label |
| the **plan against the goal** | the confirmed anchor | §17.5: deterministic tracing of every item to the anchor, a quarantined relevance call only on ambiguity, output accept / revise / ask | **unbuilt** |

Two readings of that table shape this design. The one validator that runs
live is the rule validator, and it is mostly undecided: 35 of 334 rows moved
a rule either way. And the tier that would make the others goal-relative —
plan against goal — is the one missing, which is why a run can land every
step and serve nothing. `VERIFICATION-RESEARCH.md` implications 8 and 11 name
the other two gaps: no gate can refuse to let a run end, and checks cannot be
declared anywhere but `todo`.

---

## 3. The measured record

The appraisal docs cited one pilot until 2026-09-24. The record holds more,
and the rest is less kind to guidance; every figure below is in `HISTORY.md`
under 2026-09-09 and 2026-09-10, and none is pooled with another.

| measurement | control | treatment | reading |
|---|---|---|---|
| guidance v1, 12 tasks × 3 seeds | 36/36 | 36/36 | tie; 71 check omissions, no anchor |
| guidance, harder tasks, every run anchored | **20/24** | **18/24** | 2 improved, **4 regressed**; gate rejected |
| guidance, privacy follow-up | 6/6 | 5/6 | one wrong exclusion count |
| mismatch learning | 10/12 | 9/12 | learning did not beat control |
| attribution v2 | 30/36 | 30/36 | tie |
| learning lifetime v2 | 6/6 | 6/6 | two clean reflections, below the minimum of three: no rules formed |
| executable validation, frozen rules | — | 1 improved, 2 regressed per cap | exposure, not learning |
| gossip extra peer rounds | — | no added coverage | 7 of 55 admitted claims contradicted |

Two further facts constrain every validator-based proposal: judge-graded
validation is unstable — the same inputs graded *improved*, then
*unchanged*, then *improved* (HISTORY, 2026-09-11) — so a single
directional ledger verdict is not evidence; and the structural verdicts of
`counterfactual.rs` are the ones to build on.

**The nightly learning half is healthy machinery with no input.**
`scripts/ruminate.sh` (`mecha-ruminate.timer`, 03:30 UTC) runs reflect →
distill → validate → learn `--auto` → propose-retirements → harness
ruminate. Over the five nights to 2026-09-24: no new reflection; `learn` idle
every night (all four situation batches under the minimum of three);
`validate` ran no probe on four nights (unchanged inputs are deferred) and on
the fifth ran twelve, none decisive; nothing retired; no harness proposal.
The four active rules sit at zero improved and zero regressed after 17–25
graded probes each. Every harness candidate ever proposed — twelve — was
rejected, four as "no discriminating power, all paired episodes tied", three
for keys that do not exist; none has been proposed since 2026-09-10. The
correction rate per session fell from 0.45 to 0 over four weeks. A loop that
moves only on owner corrections goes quiet exactly when the owner stops
correcting, which is either success or disengagement, and the store cannot
tell which — the operational case for collecting the verdicts in §4.

**When the model stopped planning is part of the premise.** In late August
112 of 120 appraised sessions wrote a plan (`HANDOFF.md`, a corpus that still
included development runs); the store holds no `todo` call after
2026-08-28, and 4 of 79 long real runs in the last 30 days used one. A
design that keys on a plan must not assume that changes.

---

## 4. Owner verdicts already given, and unread

The pipeline is starved of verdicts while the owner gives them daily on
surfaces the appraisal does not read. What it reads today: a draft sent
unchanged (+1.0), edited (−1.0) or rejected (−1.0); a question answered
(+0.5) or abandoned (−0.5); a front-door request closed with nothing sent
(−0.5); a steer, denial or stop (−1.0); a follow-up the reflector judges a
correction (−1.0); a task closed `done` (+0.5, printed at closure, never
stored); a post-delivery outcome (`outbox outcome`, CLI only); `run --goal`
or an answered `ask_user`.

What happens on a surface and signs nothing:

| owner act | surface | what it says | wiring proposal |
|---|---|---|---|
| reopening a task after `done` | board, TUI, web | the completion was wrong | S3a (−1.0 on the closing session; undoes L2's success) |
| closing a task in `mecha-graph tui` | graph TUI | the same verdict as `tasks set`, which it bypasses | S3a + §1.4: the closure leak |
| workflow `close` / `cancel` / `reopen` / `verify` | CLI, Today page | accepted / abandoned / wrong / checked | S3a |
| the `--reason` on an outbox reject | CLI, web | the owner's own correction, in words | S3a → reflect (L2) |
| `outbox reconcile` | CLI | whether a "sent" actually arrived | the precondition for any delivery positive (X5, S7) |
| retiring or restoring a rule; dropping or editing a reflection | web learning settings, CLI | a verdict on the learner and the reflector | S3a → L3 tenure |
| harness `accept` / `reject` / `revert` | CLI, web | a verdict on a diagnosis | S3a → L6 |
| graph review-queue verdicts on facts from `agent:mecha` episodes | graph review queue | the owner rejecting what a session claimed | S3a, via the source episode's session id |
| mail acts: `reply`, `task`, `schedule` | web mail | the owner taking on a commitment | S4 |
| snoozing or acknowledging a workflow reminder | Today page | the cost of that interruption | U1 |

---

## 5. mecha-graph: where it overlaps the appraisal system

The graph re-implements harness concerns the appraisal system also owns,
several of them better. The owner's direction (2026-09-24) is that mecha is
the harness and mecha-graph a tool that should eventually merge into it;
`APPRAISAL-WIRING-DESIGN.md` §5 turns that into a port-on-demand rule. The
last column names the wiring proposal that ports each piece.

| mechanism | mecha today | mecha-graph today | after the merge | wiring step |
|---|---|---|---|---|
| tenure moved only by owner verdicts | rule probation and retirement on a streak of attributed regressions | `ladder.rs`: staged → sampled → trusted per class on the **Wilson lower bound** of the *human* accept rate; `HUMAN_VERDICT_SQL`, `reviewed_by` | one tenure rule | L3 ports the Wilson bound for per-line rule tenure |
| attributing an owner correction | `Agency` from the channel (owner vs own), refined by a paid replay | D3 contract: *data error*, *behaviour error* or *gap*, decided by what the retrieved context held; distill's `meta.corrections` already supersedes and negates the wrong fact | one attribution | L7: `reflect` mines a behaviour lesson only for a behaviour error; a gap becomes its own class |
| deterministic verification | `grounding.rs` (`admit`, `calls`) | `verify.rs` / `kg_verify` | one grounding primitive (VERIFICATION-RESEARCH: already "hand-rolled four times") | C1 and C2 use `grounding.rs`; port `verify.rs` into it when the graph merges |
| model-free priority | replay by cost headroom, charter-rank tiebreak | Selector: demand × slot gap × staleness, SQL only, drives nightly `gossip` | one priority function | L1's *need* term is the Selector's demand term |
| usefulness | none | utility loop: retrieval touches per class, report-only | salience | M2 reads retrieval demand beside earned error |
| decay | rules retire only on regressions | `decay.rs` closes beliefs whose statistic no longer holds | dormancy | L3: a rule whose region stops recurring goes dormant |
| surfaced verdict queue | `mecha review`, outbox | review-on-use: shadow tier, verdicts ordered by contradiction → retrieval → spot-check | one queue | S2, S6, R10 and A2 confirmations land in `mecha review` |
| contested evidence | none structural | pack flags: contradicted, denied, stale — reach mecha only as JSON in a tool result | an evidence fact | anticipation `Evidence` reads a flag as `verification: Unknown` (X3, G2) |
| the board and its closure | `tasks set` runs the closure appraisal, follow-up and project closure | `gtd::set_task_status` — also reached from the graph TUI, which bypasses all three | one closure path | **fix now** (R15): the graph TUI closes through `mecha tasks set`, or the graph records a closure event mecha's nightly appraises |
| appraisal metadata on episodes | `distill` exports `meta.goal`, `meta.serves_charter`, `meta.affect`, per-error `goal` | only `meta.corrections` has a reader | read in-process | the claim was corrected 2026-09-24; salience is built where the queue lives after the merge (L5) |
| goal tier above project | `GoalRef` stops at `charter` / `project` / `task` | a `goal` node type exists; 0 goal nodes | decide once | not proposed; recorded so it is not rediscovered |

---

## 6. Designed elsewhere, ruled or proposed, and still owed

Each belongs to the document named; the wiring design depends on or absorbs
some of them. Listed so the next reader does not
rediscover them.

- **GOAL-SYSTEM-DESIGN §5.3, self-authored steers** — the agent names its
  own counterfactual ("I should have asked before staging these") and that
  point is probed like an owner steer. The retrospective half of X; see X0.
- **§17.1's goal-relative second reading of `steer_verdict`** — the signed
  error on the cited goal's computable channels between the recorded and the
  replayed trajectory.
- **§17.7 item 7's evidence-class grading of stops** — a redirect medium, a
  silence weak.
- **Disjunctive scope** — a narrowing that splits a region rather than
  retiring a rule.
- **§11.1's `board_overdue` and `cost` sensor kinds**, and "which line moved"
  on a reflection.
- **APPRAISAL-RESEARCH §3.6's third positive channel** — a read receipt on a
  trigger's briefing; **§3.8's trajectory counters** — a same-region re-edit,
  acting after declaring done, a verify claim with no exit code read;
  **§3.10** — measure the quarantined appraiser at scale, then keep or retire
  it (it returned "no further error" on 169 of 169).
- **Replay reconstruction of evidence-bearing runs** — probes refuse them
  until it exists, so X1 cannot store verdicts for them.
- **VERIFICATION-RESEARCH implications 9 and 10** — tool results carry their
  command and exit status, enabling an "untraceable numbers" check on a final
  answer (C1's natural extension); and if an adversarial verifier is built,
  a *pair* on gossip's lens pattern, never a lone critic.
- **Closure follow-up staging is not atomic**, and the closure readout reaches
  only a terminal's stderr — a task closed from the web board is appraised
  and the page never shows it.
- **EXPERIMENT-DESIGN §17's datasets** as the evaluation substrate, and
  §15's appraisal-off preset.
