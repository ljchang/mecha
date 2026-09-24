# The appraisal system as built — inventory, readers and the measured record

Researched 2026-09-24 against `main` at `b6cfa73c`, refreshed at `9eea04e7`.
One question: **what appraisal-related functionality exists across mecha and
its sibling repositories, what reads each signal, and what has been
measured?** The answer is the evidence base for
[`APPRAISAL-WIRING-DESIGN.md`](APPRAISAL-WIRING-DESIGN.md), which decides
what to build from it. The signals themselves are designed in
`GOAL-SYSTEM-DESIGN.md` and their invariants are ARCHITECTURE's goal-system
section; neither is restated.

Method: fourteen passes. Five on the first day covered a signal inventory, a map
of harness decision points, the live corpus (aggregates only,
`MECHA_SESSION_KIND=test` on every readout) and two literature reviews. Five
more covered the core goal/assistant modules, learning and self-improvement,
every owner-facing surface, the sibling repositories, and the record (HISTORY,
HANDOFF, the design docs and ~45 merged PRs). Four last passes, through
the owner's statement of purpose, covered every interpretive model pass, the
context supply, the local-model budget, and counterfactual policy
evaluation (§7–§10). Every shipped claim was
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

Doc/code disagreements found on the way. Each was corrected the same day —
on `docs/appraisal-wiring` for the code comment in `distill.rs`,
ARCHITECTURE's goal-system section and the website's appraisal pages; the
pages outside the appraisal section (the distillation, memory, learning and
workflow pages) belong to another lane and were corrected on its PR #283,
so until that merges the distillation page still carries the claim below:
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
- `Channel::Setpoint` has no production producer, and nothing in the harness
  mints a `GoalRef::Setpoint` — both exist because the enums are a wire
  format — though `GoalRef::from_str` accepts `setpoint:<id>`, so `run --goal`
  or a model's `serves` can still write one. The website's reference page
  presented `setpoint` as a live channel.

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
for keys that do not exist; the latest was proposed 2026-09-23. The
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

---

## 7. Every interpretive model pass

Fourteen places a model interprets a session, a step, a draft or content, as of 2026-09-24. Volumes are the last 30 days of the live store.

| # | pass | code | reads | writes | when | volume (≈30 d) | consumers | taint handling | retrieved into a later run? |
|---|---|---|---|---|---|---|---|---|---|
| 1 | **Distiller** | `distill::Distiller::distill`, `DISTILLER_SYSTEM` | the whole session via `render_for_distill` (the compaction summariser's prose rendering, tool results clipped hard, head+tail bounded) | `episode` (2–8 sentences, factual, past tense), `corrections` (wrong/right/about), `surprises` (predicted-from-graph / actual / about), or `skip` | nightly `ruminate.sh` (`distill -p`) | **219 sessions** in 30 d (18 in the last 7); ledger `distilled.jsonl` (session ids only) | graph: `kg_upsert` episode (the graph extracts beliefs from the prose into its review queue); `meta.corrections` → graph supersede/negate (live); `meta.affect`/`goal`/errors → nobody; surprises → printed | runs on **tainted sessions too**; taint recorded on episode meta; the graph reads episodes as untrusted | only indirectly: facts extracted from episodes, after owner review, reach runs via `kg_*` (untrusted) |
| 2 | **Reflector** (behavior / writing / **mismatch** frames) | `learning::Reflector::reflect`; `REFLECTOR_SYSTEM`, `WRITING_REFLECTOR_SYSTEM`, `MISMATCH_REFLECTOR_SYSTEM` | one `Intervention`: trigger, context window, the owner's text, aftermath, tools before/after; mismatch frame adds bounded criterion diagnostics | `Reflexion`: `reflexion_text` (1–3 sentence directive), `error_type`, `confidence`, `goals`, `situation`, provenance | nightly (`reflect -p`) **and** per session (`learn-live.sh`) | 39 reflections in 30 d; `mined.jsonl` 218 sessions mined in 30 d (most yield 0 interventions) | `learn` (rules), `goal_lessons`/`goal_context`, validation probes | reflection carries `Origin` from recorded taint; learn excludes non-clean structurally | yes — through learned rules in the prefix and `goal_context` |
| 3 | **Learner** (behavior / writing / triage) | `learning::Learner::learn`; `LEARNER_SYSTEM` etc. | a region batch of reflections + that region's existing rules | rewritten rule set for the region | nightly `learn --auto`, live-learn | idle 5 of 5 recent nights (batches below min 3) | `rules_carried_for` → prompt prefix | clean reflections only | yes — the prefix |
| 4 | **Compaction summariser** + **summary validator** | `Agent::compact` (agent.rs), `compact::SUMMARY_SYSTEM`/`SUMMARY_INSTRUCTION`, `validate_summary` | the live conversation rendered as prose | a summary that replaces history; validator: omissions verdict → regenerate | in-run, when context crosses threshold | **0 compactions** in 30 d of real use | the same run | stays in the run; taint survives compaction | n/a (in-run) |
| 5 | **Step escalation** | `Agent::escalate_step`, capped `MAX_STEP_ESCALATIONS_PER_RUN` | a step's span + deterministic finding | JSON verdict → revise-plan nudge | in-run, plan-gated | **off** by default; 0 plans live | the same run | in-run | n/a |
| 6 | **Quarantined appraiser** | `appraisal::appraise_with_model`, `APPRAISER_SYSTEM` | `AppraiserEvidence` — counts, channels, label, goal-named bit, pressure, load; **no text** | one extra `GoalError` (appraisal channel) or none | on demand (`sessions appraise --appraise`) | ~0 live; measured once: 169/169 "nothing further" | the readout | typed, counts only | no |
| 7 | **Anticipation** | `anticipation::assess` | owner/harness `Evidence` | kinds + `Response` | staging, plan writes | 6 predictions, 0 outcomes | outbox guide gate, `Decision` | owner/harness evidence only | no |
|   | *(deterministic — not a model pass)* | | | | | | | | |
| 8 | **Gossip** readers + vet judge | `gossip.rs` (`LensedSearch` children; `vet_judge`) | graph search results through one lens each; judge sees handed evidence only | contradictions, corroboration, adjudications | nightly via mecha-graph `nightly-mecha.sh` (Selector) | 83 log rows over the last ~30 log days | the graph | readers are full agents with one lensed tool; judge quarantined | through the graph |
| 9 | **Diagnostician** | `diagnose.rs` (+ `commands/harness.rs`) | `diagnose::Evidence` — counters, means, sensor lines | a typed `HarnessCandidate` | nightly `harness ruminate`, only when a detector trips | "no proposal tonight" every recent night; 12 candidates ever, the latest 2026-09-23, all rejected | `candidate::judge` | numbers only | accepted candidates → config override layer |
| 10 | **Mail triage classifier** | `mail_triage.rs` (`QuarantinedPass` at the classify call) | one inbound message (**third-party**) | typed verdict: bucket, urgency, action, tags, deadline, request kind | per new thread, through the working day | the largest-volume pass | the mail surface, the digest | quarantine is the whole security argument; deadline quotes grounded | typed fields only |
| 11 | **Mail corrections reflector** + **triage learner** | `commands/mail.rs` (`reflect`), `TRIAGE_LEARNER_SYSTEM` | the owner's corrections to triage verdicts (with the mail they concern) | triage lessons → triage rules | on `mecha mail reflect` / nightly | small | the triage classifier's prompt | "mail it reads is the least trusted input in the system" | into the classifier only |
| 12 | **Front-door extractor** | `frontdoor.rs` | a stranger's request prose (**third-party**) | typed `Record` (dates grounded) | per request | 12 requests all-time | triage run via `for_privileged_run` (typed, never prose) | quarantine | typed fields |
| 13 | **Title generation** | `title.rs` | **the owner's own turns only** | a session title | as a conversation grows | per active session | session lists on every surface | owner turns only — its security argument | display only |
| 14 | **Eval judge** | `eval.rs` | a case's output vs rubric | grade | `mecha eval` only | experiments only | scorecards | quarantined | no |

Not model passes, but interpretive inputs any convergence must read: the
signed-error assembly (`appraisal::of_session`, `attribute_events`), the
grounding checks, the probe verdicts (`counterfactual.rs`,
`appraisal_probe.rs`), the step findings (`step::appraise`).

### What the picture says

- **Three passes read the same session, at the same moment, for different
  consumers**: the distiller (graph memory), the reflector (lessons), and the
  appraiser (the label). They already run in the same nightly job
  (`ruminate.sh`: `reflect -p`, `distill -p`), and the reflector also runs per
  session. None sees what the others concluded.
- **The distiller is already an interpreter of meaning.** It reads the whole
  session, writes prose, and records `surprises` — predicted vs actual, which is
  a prediction error — and `corrections`, which the graph already acts on. It
  runs on tainted sessions and records the taint, which is exactly R18's shape.
  It lacks the context the owner names: it is never shown the run's goal, the
  charter, the homeostatic state, the owner's later acts on the output, or past
  interpretations of the same situation.
- **The appraiser is the pass with the least to work with.** Numbers only;
  169/169 "nothing further".
- **The reflector is the only one whose output reaches later runs**, and it
  sees one intervention window, not the session or its goal.
- **Six passes must stay separate** by function or by security: the
  third-party interpreters (mail triage, the front door, the mail corrections
  reflector — quarantine is their safety argument, and their output crosses
  as typed fields only); the in-run passes (compaction, step escalation — they
  act inside a live run under its budget); gossip (its value is two
  *independent* readers); the titler (owner turns only, by design); the
  diagnostician and the learner (they consume interpretations across many
  sessions and author a different artifact); the eval judge (measurement).

---

## 8. The context supply: goals, system state, past experience

"Prompt-eligible" means it may reach a model prompt under today's invariants: nothing per-turn in the prefix (§4.3); sensor numbers and setpoints never in a prompt (containment 2); third-party prose only through the quarantine or a typed extraction.

Two delivery precedents already exist for per-run described state and are the
templates: `date_context::render` folds a clock reading **into the outgoing
user message** whenever it stops being true (never the prefix), and the `todo`
result carries the **headroom line** (`pressure::Forecast` Display) — the one
tool-result slot where a reading changes a decision. The outbox staging result
already carries a fixed `Anticipatory guidance` line (with `goal_guidance`).

### Goals

| source | symbol / location | when available | cost | trust | prompt-eligible? | described state it can give |
|---|---|---|---|---|---|---|
| confirmed anchor | `Conversation::goal_anchor`, `GoalTrack` | run start, mid-run | in-process | owner-confirmed pointer | pointer yes | "this run serves task X / charter line Y" |
| board task (delegated) | the row `tasks work` already read; `work_prompt` | run start (delegated) | in hand | owner-created; name may be captured prose | already in the seed (name, due, overdue, waiting_on, project) | "due tomorrow, overdue, waiting on a person" |
| board (all open tasks) | `kg_task_list` via the graph MCP server | run start via one MCP call; nightly | subprocess round trip | graph server marked **untrusted** — reading it in a run arms taint | counts and pointers only; names only for the run's own task | "5 next actions due this week; 2 overdue" |
| projects | board `project_id`; `appraise_project` | as board | as board | as board | pointer | "part of project P, 3 tasks open" |
| charter lines + sensors | `Charter::load`, `charter::prompt_block_for`; `reading::read_line` | run start (in prefix already) | in-process | owner-authored, arms nothing | text yes (already); **sensor setpoints/readings no** | line text; per-line band ("this line's store is overdue") only as words |
| triggers | `TriggerStore`, the trigger's own name/prompt | run start (trigger runs) | in-process | owner-authored | name/pointer yes | "the morning brief" |
| workflows / commitments | `workflow::Commitment`, `Workflow::checks`, store `~/.mecha/workflows` | any | in-process | owner-established | pointers, party, due | "you promised X to Y by Friday" — **store absent on this install** |
| parked questions | `questions::QuestionStore` (`Question`: status, question, goal/serves) | any | in-process | the question is model-authored text; the answer is the owner's | pointer + owner answer | "1 question waits on you, 29 days" |
| front-door requests | `frontdoor` records, `WAITING_ON_OWNER` | any | in-process | stranger content — typed fields only (`for_privileged_run`) | typed fields only | "a meeting request awaits triage" |
| mail-triage verdicts | `mail_triage::Verdict` (bucket, urgency, deadline, request_type), `~/.mecha/mail-triage/*.json` | any (store on disk) | in-process file read | quarantined typed extraction of third-party mail | bucket/urgency/deadline pointers only, never `reasoning` prose | "3 threads marked respond-today" — a *claim*, never a commitment |
| calendar | mecha-mail MCP (`calendar_*`) | run start via MCP call | subprocess + OAuth; seconds | event text is third-party → untrusted | only a computed band, narrowing uses only | "your afternoon is booked solid" — **no harness reader exists** |

### System state

| source | symbol | when | cost | trust | prompt-eligible? | described state |
|---|---|---|---|---|---|---|
| homeostat at start | `Homeostat::at_start` (load_avg, mem, backlog, charter readings) | run start | in-process | harness-measured | bands only | "machine is busy" |
| backlog / attention debt | `Backlog::read` (outbox, questions, frontdoor `Depth`: waiting, oldest) | run start; any | in-process | harness | bands/small counts | "8 drafts have waited over a week" |
| per-commitment guilt | `guilt::anticipated_guilt` (scalar, saturated); per item after 1e/1f | run start | in-process | harness, recorded-only | never the number | "two replies to people are overdue" |
| context pressure / forecast | `pressure::ContextTracker`, `Forecast` (`turns_left`), `ToolCtx::context` | mid-run, per request | in-process | harness | bands (already on the `todo` result) | "context two-thirds used; ~4 turns left" |
| turn / token budget | `AgentConfig::max_turns`, `max_tokens`, cost budget | run start / mid-run | in-process | config | yes as words | "3 turns left in this run" |
| background seats | `permit::Permits::live()`, `capacity()` (`~/.mecha/permits`) | any | in-process dir read | harness | yes as words | "all background seats busy" |
| runs in flight | `runmarker::RunMarkers` (`live_writer_of`), `~/.mecha/taskruns`, trigger ledger | any | in-process | harness | counts/pointers | "another delegated task is running" |
| llama-server load | `/props` known; `/slots` not consumed (GOAL §4.2) | — | HTTP call | harness | bands | **gap**: slot occupancy has no reader |
| voice call in progress | voice takes no permit (`permit.rs` doc) | — | — | — | — | **gap**: only `serve` knows |
| quiet hours / digest | `workflow::AttentionPolicy` (timezone, quiet_start/end, digest_hour; default when file absent) | any | in-process | owner config | yes | "it is inside your quiet hours" |
| local time | `clock::Clock` + `[agent] timezone`; `date_context::render` | per turn (already folded) | in-process | harness | yes (already on the user turn) | "Thursday evening, your time" |
| owner presence | last owner activity across surfaces | — | session-store scan | owner | words | **gap**: not computed anywhere |

### Past experience

| source | symbol | when | cost | trust | prompt-eligible? | what it gives |
|---|---|---|---|---|---|---|
| learned rules | `LearningStore::rules_carried_for(RUN_DOMAINS, &Situation)` | run start (prefix, stable within run) | in-process | clean-provenance only | yes (already) | standing lessons for this situation |
| reflections | `LearningStore::reflexions` (`reflexion_text`, `goals`, `situation`, origin) | any | in-process | clean for learning | clean only | lessons per correction; **0 of 66 carry goals** |
| goal lessons / examples | `ToolCtx::goal_lessons`, `goal_examples` (`learning::goal_lessons`, `planning::examples`) | run start snapshot, on demand via `goal_context` | in-process (examples scan ≤32 transcripts) | clean | yes, on demand | empty live |
| distilled episodes | graph episodes (`distill`), `kg_search` / `kg_timeline` | on demand via kg tools | MCP | untrusted by server marking | as tool results (arms taint) | narrative of past sessions, surprises, corrections |
| session corpus | `runlog::Corpus::scan`, `Session::load` | nightly (scan of ~200 transcripts is the doctor/guilt budget) | heavy | per-session taint | no (offline only) | outcomes per situation, pass^k per region |
| current transcript recall | `recall` tool | mid-run | in-process | per conversation | yes | earlier turns of this conversation |
| validation ledger | `validations.jsonl` (per rule, region, verdict) | nightly / any | in-process | harness | no (offline) | which lessons held where |
| outbox verdict history | `OutboxStore` items (sent/edited/rejected, `args_before`, reject reason, predictions/outcomes) | any | in-process | owner verdicts; drafts are model text | verdict facts + owner's reason yes; draft prose from tainted runs no | "your last two drafts to this person were rewritten shorter" |
| prior attempts on a task | board sessions + sessions store | run start (delegated) | in-process + board | mixed | pointers | **gap**: `work_prompt` carries none (M5) |
| counterfactual verdicts | X1 records | — | — | — | — | **gap**: not stored today |
| inferred owner goals | I4 store | — | — | — | — | **gap**: does not exist |

### Gaps

- **Calendar busyness** — needs a harness reader over mecha-mail; untrusted
  source, so only as a band with narrowing uses.
- **Competing-task count at run start** — needs one board read (graph MCP,
  untrusted-marked server → arms taint if done inside a run; do it in the
  harness before the run and pass counts/pointers only).
- **llama-server slot occupancy** (`/slots` unconsumed), **voice call in
  progress** (only `serve` knows), **owner presence / last activity** across
  surfaces.
- **Per-item commitment readings** (1e/1f), **counterfactual records** (X1),
  **inferred owner goals** (I4), **prior attempts per task** (M5).
- **Workflow store absent** on this install — commitments/checks exist in code
  only.

### The constraint that shapes all of it

The graph MCP server is registered untrusted, so any board or episode content
read *by a tool call inside a run* arms taint. A situation brief built by the
harness **before** the run, from in-process stores and a harness-side board
read reduced to counts and pointers, adds no taint — which is the argument for
assembling the run-start brief in `setup`, not asking the model to fetch it.

---

## 9. The local-model budget

M = measured on 2026-09-24 (GET probes, the journal, logs, session records); I = inferred from those.

### The server
- M: :8080 llama-local = qwen3.6-35b-a3b (Q4_K_M, MTP speculation), total_slots 4, n_ctx 262,144 per slot (-c 1,048,576 -np 4), reasoning budget 4096, max_tokens 8192. All 4 slots idle at probe time.
- M: :8081 llama-embed = harrier-oss-v1-0.6b, 4 slots (embeddings only).
- M: permits: DEFAULT_BACKGROUND_PERMITS = 3 of 4 seats — one seat always free for the owner's turn.
- M: config: default_provider local; [providers.anthropic] claude-opus-5 defined, no `fallbacks` configured; ANTHROPIC_API_KEY is set in the interactive shell (service env not checked). Other local providers (gemma 4b on :8081 per config — note :8081 currently serves harrier, a mismatch worth a look; gemma26 :8082, qwen38) not probed.

### Throughput
- M (journal, last 48 h, llama-local): prefill on prompts >2k tokens: median 1,756 tok/s (p10 1,554, p90 1,873). Generation on >100-token replies: median 93 tok/s (p10 71, p90 102) — mostly single-stream.
- M (LLAMA-SERVER.md, 2026-08-20): single-stream 83–85 tok/s at -np 4; 4-stream aggregate 135–140 tok/s (1.6×, bandwidth-bound; speculation dilutes under batching). So ~45 tok/s per stream when three run at once (I).
- M: prefix cache works per slot — a follow-up turn prefills only new tokens (48–106 tokens, ~0.3 s) vs a cold 14.7k-token prefill in 7.7 s.
- M: request volume on :8080 ≈ 2,333 in 48 h (~1,170/day). By UTC hour: 01–05 (graph nightly 01:30, ruminate 03:30, mail classify 05:33) and 08 (graph gossip) and 20–21 dominate; 12–18 UTC is light. Interactive real runs are ~8/day (245 in 30 days).
- M: nightly ruminate wall time over 14 nights: 1–25 min (median ~14 min), mostly the harness-measurement stage; reflect/distill/validate/learn are near-idle (live hooks do most distilling at session end: 587 sessions distilled so far).
- M (session records since 2026-08-25, 144 real runs with duration): run wall time p50 11 s, p90 57 s, max 420 s; web p50 10 s / p90 42 s; trigger p50 50 s / p90 67 s; voice p50 9 s. Peak prompt tokens p50 27.8k, p90 64.5k. Output tokens p50 409, p90 2,212.

### Cost of each proposed pass (I, from the rates above)
- **I1, one interpretation per session (quarantined one-shot, cold):** transcript flattened and clipped ~10–30k tokens → prefill 6–17 s; thinking up to 4,096 + output ~800 tokens at 45–93 tok/s → 10–60 s. **≈ 20–75 s per session**, one background seat.
- **I3, a situation appraisal at a boundary:**
  - as a *separate cold pass*: prefill the run so far (p50 28k, p90 65k → 16–37 s) + 5–15 s generation with thinking capped → **≈ 20–50 s** per boundary. That doubles a p50 trigger run and quintuples a web turn.
  - as a *turn in the run's own slot* (shares the cached prefix — the run is already tainted, R22): prefill ~0.3–1 s + generation 5–15 s → **≈ 5–15 s** per boundary. This is the affordable shape. (Slot affinity for a second request is inferred from llama-server's longest-prefix slot choice, not measured.)
- **Online K-rollout policy evaluation** (K continuations of ~3 turns each from the current point):
  - Seats: K ≤ 3 without taking the owner's seat; K=4 is not available on this box while anyone might type.
  - Each rollout needs the prefix prefilled in its own slot (p50 28k → ~16 s each, compute-bound, partly serialised) and ~3 × (400 out + up to 1k thinking) ≈ 4k tokens at ~45 tok/s under 3-way batching ≈ 90 s.
  - **≈ 1.5–3 min per decision point at K=2–3**, holding every background seat for that time (mail classify, triggers, duty runs queue behind it), plus a validator: structural checks ≈ free; a judge call ≈ +20–40 s.
  - **The binding constraint is not tokens but tools.** A rollout cannot execute real side-effecting tools; it needs recorded results (replay) or read-only/staged tools. The nightly log shows the limit already: replay arms "diverged — dropped" as soon as behaviour changes, because there is no recorded result for a new call.

### Budgets
- **Nightly (offline counterfactual, reflection, I1):** the 03:30–05:30 UTC window before mail classify is ~2 h × 3 seats ≈ 360 episode-arms of median trigger size, against ~32 arms used by ruminate on a busy night — **~10× headroom.** I1 over every day's sessions (~6–10/day) is ~5–10 min of one seat — trivial; even at-session-end (as distill already runs) it is within the background permits.
- **Daytime:** 12–18 UTC is lightly used; background work there must stay model-idle-gated (`scripts/model-idle.sh` reads /slots) and yield to voice (the 2026-09-04 diagnosis found a title one-shot competing on the GPU during a call).
- **Per long run (I3):** 2–3 in-slot boundaries ≈ +10–45 s — acceptable on delegated/trigger runs (p50 50–60 s), not on short web/voice turns, which should get none.
- **Online K-rollouts:** feasible only on delegated/unattended runs, at 1–2 decision points, K=2–3, with replayable or read-only tools; never on interactive or voice turns.

### Preemption and fallback
- Interactive-first is structural already (3 of 4 permits). Anything new — I1 at session end, K-rollouts, nightly counterfactuals — must take a permit and yield to a voice call or a waiting owner turn.
- Cloud fallback: an anthropic provider is defined but no `fallbacks` are set, and the design rule is "strict beats silently answering with a different model". Using it to absorb I1 or rollouts would be an explicit per-pass provider choice — and would send transcripts off-box, a privacy/trifecta decision for the owner, not a capacity knob.

---

## 10. Counterfactual policy evaluation: machinery, validators, feasibility

### What exists, what it can evaluate, what it costs, how it fails

| machinery | evaluates | cost | known failure modes | online? |
|---|---|---|---|---|
| `replay_run::drive_branch` / `branch_at` | one continuation from a recorded point: the recorded prefix is resubmitted verbatim (reads from KV cache on a caching server) and the model generates only from the point; tools answer **from the recording** | one model continuation per arm | after the continuation leaves the recording, every recorded answer "answers a question nobody asked" → `OnDivergence::Stop` (default). **There is no world model past divergence.** `OnDivergence::Live` falls through to real tools — which "can execute real tools", unsafe for sends | the mechanism is generic (a branch from a transcript prefix); the *recorded-tool* registry is what makes it offline-only |
| `probe::drive_arm`, `prepare_probe_in` | K arms (system prompts / rule sets) from one prepared prefix; `validate` already bisects over more than two arms | one continuation per arm | refuses evidence-bearing runs (no reconstruction); refuses missing tool surfaces | no |
| `counterfactual.rs` verdicts (`ProbeVerdict::{Pass, Fail, Inconclusive}`) | **steer**: does the continuation track the recording from the steer point without the steer; **denial**: does it avoid repeating the refused call; follow-up: judge-graded (`--judge-provider`) | per point | structural verdicts are sound but narrow (only "did it do what the owner steered to"); follow-up grading is a judge; before surface fidelity 12/13 probes were inconclusive | no |
| `appraisal_probe` (`sessions appraise --probe`) | was a steer load-bearing → regret vs disappointment | one model run per steer, budgeted | computed on demand and **discarded** | no |
| `harness_probe` + `candidate::judge` | a config candidate vs current harness, whole-session paired replay; prioritised selection + uniform holdout; `MIN_SELECTION_PAIRS 8`, `WORK_FLOOR 0.75` | 2 × sessions model runs per candidate, nightly | **12/12 candidates rejected**, 4 as "all paired episodes tied"; the latest proposed 2026-09-23. Whole-session replay cannot measure a behaviour-changing candidate: divergence drops the pair (SELF-IMPROVEMENT §14.6). Selection by cost-metric headroom picks uninformative episodes | no |
| `mismatch::ArtifactCase` + `probe::prepare_mismatch` | whole task re-run in a fresh directory against pinned gold, per arm | a full task per arm | only builtin file tools; no live services | no |
| `experiment.rs` arms / lifetimes, `trial_env`, fixture servers, synthetic home, AgentDojo source | whole-policy comparison in isolated homes with artifact graders, including behaviour-changing levers | many full runs | small N; variance; the 09-09 pilots tied or regressed | no (offline instrument) |
| `surface::Fidelity` | whether a recorded tool surface still exists | — | precondition for any replay verdict | — |
| sandbox (bwrap/docker, `--tmpfs /tmp`), `work.rs` dirs, `PermissionMode::ReadOnly` | confinement; a read-only run mode already exists at the approver | — | **no workspace snapshot / copy-on-write exists**; ARCHITECTURE's mismatch paragraph: branching "a filesystem snapshot that was never captured" is deliberately not done | the building blocks for dry branches exist except the snapshot |
| `subagent.rs` | a fresh `Conversation` with an allowlisted registry; inherits taint | one child run | returns prose; no comparison built in | yes — the one online "alternative trajectory" mechanism today |
| `questions.rs` park | end a delegated run with the ball on the owner | — | — | yes |

**The diagnosis.** Every evaluator mecha has compares a trajectory against *recorded evidence*, so it can only judge moves that stay on the recording (did it do what the owner asked; did it avoid what the owner refused). A genuinely different policy leaves the recording within a call or two, and at that point there is no world to evaluate it in. That — not the gate — is why rumination starves: the arms tie because they are compared where nothing differs, or they diverge and drop.

Two worlds exist in which a different policy *can* be evaluated: **fixtures** (the synthetic home, the task suites, AgentDojo — reproducible offline) and **the live world, read-only** (online: reads are real, writes and sends must be made dry).

### Which validators may decide between two policies

| validator | trustworthy to decide? | where |
|---|---|---|
| owner's recorded verdict (release/edit/reject, closure/reopen, steer target) | **yes** — ground truth, offline only | point-wise offline comparison |
| owner-authored executable checks (`Workflow::checks`), artifact criteria against pinned gold | **yes** | fixtures offline; live runs when the owner wrote them |
| structural counterfactual verdicts (steer tracks, denial not repeated) | **yes**, for the narrow question they ask | offline replay points |
| grounding (`grounding::admit` — claims dereference into what was received) | **yes**, as a veto (ungrounded claim loses) | online and offline |
| V1 tracing (plan items trace to the anchor) | **yes**, as a veto on scope, says nothing about quality | online plan comparison |
| model-declared frozen checks | **one-sided**: a failure disqualifies; a pass does not prove | online |
| budget fit (open steps vs turns left, predicted context) | **yes**, as a feasibility filter | online |
| owner-verdict predictor (X3 base rates) | **rank only**, never decide alone | tie-break |
| model judge (LLM-as-judge, quarantined) | **no** — measured unstable locally (same inputs graded improved/unchanged/improved, HISTORY 2026-09-11); judges anchor on confident closing language (τ² AUROC ≤ 0.65, arXiv 2606.09863); an imperfect verifier's false-positive rate rises with K (VERIFICATION-RESEARCH, *Inference Scaling fLaws*) | never the deciding vote; at most a tie-break among candidates that passed every structural validator |
| the model's own report | **never** | — |

Consequence: online comparison is only worth doing where a structural validator exists for the decision at hand — a declared or owner check, grounding, tracing, budget fit. Where none exists, branching buys nothing but cost (the "verification gap").

### Online feasibility

**Dry-able action classes.**
- Read-only tools (`Tool::read_only`, the approver's `PermissionMode::ReadOnly`): safe to execute live in a branch. They cost time and, if external, arm taint.
- Sends and publishes: already dry — outbox staging never delivers. A branch stages into a scratch outbox that is discarded if the branch loses.
- File writes: need a **scratch copy of the jail** (new: a copy or overlay of the run's workspace per branch; workspaces under `~/.mecha/work/<producer>/` are small). Commit = copy the winning branch's diff back, or re-execute its writes.
- Shell: runs in the sandbox against the scratch copy; with no network when confined.
- `Egress::Chosen` calls, calendar holds, anything irreversible without a staging route: **not branchable** — the branch halts before them and the comparison ends there.

**Triggers for a mid-run policy change** (all harness-computed, none self-report): a failed frozen or owner check; the frustration ladder reaching rung 3 (repeated own failure on one target); a surprise (forecast overrun ×3, a check failing after a step marked done); budget shortfall (open steps > turns left); drift (V2 — plan no longer traces). At plan time: a delegated or trigger run with an anchor, before the first write.

**Resources.** llama-server runs `-np 4` (four slots, ~84 tok/s single stream, ~138 aggregate; LLAMA-SERVER.md) with `kv_unified = false`, so a branch on another slot re-processes the shared prefix rather than reusing it. Interactive work preempts background (R1 of TASK-AGENT). So online branching is affordable at **K = 2** (at most 3), bounded horizons (a plan, or ≤ N tool calls), on delegated/unattended runs or when a slot is free — never stalling an interactive turn. The literature agrees on small K: optimal K ≤ 5 at cost ratio 4 and K = 0 at ratio 10 (fLaws, via VERIFICATION-RESEARCH).

**How results flow back.** The winner continues; the loser is not discarded but summarised — "the alternative (…) failed its check / left the anchor / ran out of budget" — into the run's appraisal (I3) and the episode's interpretation (I1), so a losing branch still teaches. Taint: every branch inherits the conversation's taint; because the loser's outcome is fed back as text, the conversation takes the **union** of all branches' taint (conservative — unknown is never clean).

### Literature (2023–26)

- **Tree search in the real environment** — best-first search over web actions: VisualWebArena +39.7% relative (26.4% absolute), WebArena +28.0% (19.2%); "performance scales with increased test-time compute" ([arXiv 2407.01476](https://arxiv.org/abs/2407.01476), abstract). Requires a resettable environment — mecha's analogue is fixtures offline, dry branches online.
- **LATS** (MCTS + LM value + reflection + environment feedback): HumanEval 92.7% pass@1, WebShop 75.9 ([arXiv 2310.04406](https://arxiv.org/abs/2310.04406), abstract); VERIFICATION-RESEARCH notes no retrievable cost accounting.
- **World-model lookahead (WebDreamer)**: the LLM simulates action outcomes before executing; competitive with tree search at "4–5 times more efficient" on VisualWebArena; works on live sites ([arXiv 2411.06559](https://arxiv.org/abs/2411.06559), abstract). The model grades its own imagined futures — mecha would need a structural validator on top.
- **General agents do not scale**: across ten leading agents, "neither scaling methodology yields effective performance improvements in practice" — sequential scaling hits a **context ceiling**, parallel sampling a **verification gap** ([arXiv 2602.18998](https://arxiv.org/abs/2602.18998), abstract). The single strongest argument for gating online branching on a structural validator.
- **Verifier limits** (already in VERIFICATION-RESEARCH): repeated sampling converts to accuracy only with an automatic verifier (Large Language Monkeys); an imperfect verifier's FPR rises with K and caps gains; hybrid execution + execution-free verifiers beat either alone (R2E-Gym); Anthropic's agentic best-of-N discards patches that break visible regression tests first (+6.6 to +7.5 pp).
- **Rollback + reflection**: Rollback-Induced Reflection restores a prior state while keeping a structured reflection from the abandoned trajectory; "consistently improves average task performance across multiple LLM backbones" on three long-horizon benchmarks ([arXiv 2609.18304](https://arxiv.org/abs/2609.18304), abstract; no figures in the abstract — UNVERIFIED magnitude). Directly the "loser teaches" flow in §3.
- **Speculative actions** (latency, not quality): a fast model predicts next actions executed in parallel, committed only when they match; up to 55% next-action accuracy, up to 20% latency reduction ([arXiv 2510.04371](https://arxiv.org/abs/2510.04371), abstract). Lossless only for side-effect-free steps; relevant later, for P2/performance.
- Searched, not read: FineVerify ([2606.00660](https://arxiv.org/pdf/2606.00660)), Multi-Agent Verification ([2502.20379](https://arxiv.org/pdf/2502.20379)), budget-aware discriminative verification ([2510.14913](https://arxiv.org/pdf/2510.14913)) — UNVERIFIED.
