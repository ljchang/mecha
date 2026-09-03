# The appraisal system — review, corpus measurement, and what the literature says

Researched 2026-09-02, against `main` at `102bacc`. One question: **why is the
appraisal system still inert a week after rungs 0–10 shipped, and what would
make it an effective feature rather than a recorded one?** The design is
`GOAL-SYSTEM-DESIGN.md`; the invariants are `ARCHITECTURE.md`'s goal-system
section; this file does not restate either. Where a finding here argues
against the design, it says so beside the section it argues with.

Three sources: the code (`appraisal.rs`, `guilt.rs`, `step.rs`, `boredom.rs`,
`questions.rs`), the live store (`~/.mecha/sessions`, the outbox, the question
and front-door stores, the board), and two external research passes — one over
computational appraisal models (OCC, Scherer's component process model,
EMA, Soar-Emote/PEACTIDM, WASABI, FAtiMA, and 2023–26 LLM work), one over how
other harnesses grade a run with no human label. Sources are linked in §5 and
§6; anything the pass could not verify is flagged there.

---

## 0. The finding in one paragraph

The safety shape is right and should not move: the label is derived, every
cite is a pointer, dispositions may only narrow, and the literature vindicates
each of those choices (§5.3, §6.1). What is wrong is narrower and cheaper than
it looks. **The readout gates on the most expensive dimension it has and
throws away the cheapest one.** `label_of` gives an owner- or self-caused
negative error no word until `controllable` is filled, and `controllable` is
filled only by a paid replay — so the one thing the record already knows,
the *sign*, never reaches a surface. Every computational appraisal model
reviewed puts its one gate at relevance and then labels from two variables;
mecha gates at every dimension, which is exactly the product-over-unfilled-
dimensions collapse Marinier, Laird and Lewis rejected by name. Behind that,
the corpus the rung was built to measure cannot answer the question: a third
of it is development smoke tests, the middle-tier trigger has never fired,
and one sensor reads a constant. §3 lists what to change, in order.

---

## 1. What the live store says today

Read with `mecha sessions appraise --days 30 --json`, `mecha sessions health
--days 30 --json`, and a scan over the session JSONL that reads what the
appraisal does not. Every figure below is from that scan on 2026-09-02; the
scan script is not kept, because each number is reproducible from the
commands or from the fields named.

| quantity | value |
|---|---|
| sessions read / with an outcome record / appraised | 468 / 143 / 143 |
| labels | 142 neutral, 1 anger |
| sessions naming a goal (`serves:`) | 0 |
| signed errors: intervention / edit / counter | 13 / 18 / 1 |
| of which positive | 12 (all `SentUnchanged`) |
| runs / tool calls in 30 days | 250 / 710 |
| sessions from a mecha checkout, scratch dir, or eval workspace | 46 of 143 |
| sessions carrying at least one `Interrupted` run | 30 |
| sessions whose final message is an unanswered user turn | 8 |
| sessions with more than one user turn | 50 |
| board tasks by status | inbox 10 · next 11 · waiting 8 · **done 0** |
| outbox messages: rejected / sent unchanged / sent edited / pending | 22 / 12 / 2 / 7 |
| questions: open / answered / abandoned | 3 / 3 / 1 |
| front-door requests: closed / answered | 5 / 1 |
| `anticipated_guilt` over the 19 runs that sensed it | min 0.953 · median 0.959 · max 1.0 |
| `backlog_delta` non-zero | 18 of 68 runs |
| reflections in the learning store, by trigger | followup 22 · steer 16 · denial 3 · edit 2 |

Six readings of that table, each of which is a finding rather than a tuning
problem:

**1.1 The sign is present and discarded.** 22 owner-rejected drafts are the
largest owner verdict in the store, and every one reduces to `Neutral`,
because a rejection is `Agency::Owner` with `controllable: None`. The record
knows these runs went badly; only the discrete label does not. §16's open
question — discrete or dimensional — has been answered by the corpus.

**1.2 The corpus is contaminated, and nothing marks it.** 46 of the 143
appraised sessions ran from a mecha checkout, a Claude scratch directory or
the eval workspace; the rejected drafts' own reasons mostly name a self-test
batch, an injection probe, or a confirm-and-send flow test. The session
`meta` record carries provider, model and workspace and no *kind*, so the
only way to tell a smoke test from use is a path heuristic. The instrument
built to test whether the labels are degenerate is measuring the harness's
own test runs.

**1.3 The middle tier has zero events.** §5.4 calls the owner's closure of a
board task "the sharpest learning signal available". No task on the board has
ever reached `done`, so `appraise_closure` has fired zero times in
production. Rung 8's mechanism is unexercised rather than unbuilt.

**1.4 `Interrupted` is three things.** A parked `ask_user` question ends the
run through `ToolCtx::cancel` (`questions.rs`), so it records as
`StopCause::Interrupted` — the same variant as Ctrl-C, a voice-pipeline
restart, and a shutdown. Of the 30 sessions carrying one, most are
"interrupted, then a new completed run in the same session" (the owner
stopped it and re-prompted, which is a steer with no text), six end on an
unanswered user turn (abandonment), and at least one is a delegated run
correctly parking a question (`task-a281ebb5`'s session). The appraisal skips
all three on doctor's rule that an interrupt is an attentive owner. That rule
is right for the park and wrong for the other two: the 20,574-session study
of coding-agent failures operationalises failure *entirely* from re-prompts,
reverts and interruptions (§6.2), and here they are the largest unread
channel after followups.

**1.5 The guilt sensor is a constant.** `anticipated_guilt` sits between
0.95 and 1.0 on every run that recorded it, because its age and count terms
read the standing backlog — a front-door request from 2026-08-13 and seven
waiting drafts — which is a property of the *store*, not of the run.
`guilt.rs` already documents having widened the age horizon twice for this
exact reason; the third widening would find the same thing. The quantity
with variance is beside it: `backlog_delta` is non-zero on 18 of 68 runs and
says whether *this run* added to or reduced what the owner is waiting on.

**1.6 The followup channel is judged already, and the judgement is unread.**
Followups are excluded from the appraisal outright (86% of interventions, no
counterfactual). But `reflections.jsonl` holds 22 followup-triggered
reflections the reflector judged to be corrections, each keyed by session id
and provenance-gated. That is a *judged verdict* with the same standing as
the probe's, and `of_session` does not read it.

### 1.7 Validity against an independent verdict (2026-09-03)

Step 0 of `EXPERIMENT-DESIGN.md` §19: the readout joined to a verdict it
had no hand in. **The readout separates pass from fail only on the
ceiling channel, and sees a fifth of the failures.** Over 169 kept
Terminal-Bench trials (94 fail, 75 pass; four Harbor runs, mecha
0.1.0–0.1.3, `jobs/mecha-arm64-subset/`), `Valence::negative` from
`of_session` has an AUROC of 0.57 against Harbor's test-script verdict
(bootstrap 95% CI 0.52–0.62), 75 of the 94 failures carry no signed error at
all, and the label is `neutral` on all 169. The figures come from
`scripts/appraisal-validity.py`, which is kept — unlike the scan behind the
table above — because §19 wants this re-run on every corpus that has a
verdict. Read in three layers, because the first one is a finding on its
own:

| layer | what was read | result |
|---|---|---|
| the readout as shipped | `mecha sessions appraise --json` over the 172 transcripts verbatim | **appraised 0 of 172** — no session carries an `outcome` record; `Record::Outcome` landed in v0.1.7 (2026-08-19), after every one of these ran |
| the counters, reconstructed | tool calls and errored results from the message blocks, compactions from `rewrite` records, turns and usage from `summary`, the stop cause from Harbor's exception record joined to the transcript's `max_turns` | consistent with Harbor's own record on every row: all 20 `AgentTimeoutError` trials are exactly the 20 transcripts with no `summary` |
| `of_session` over the reconstruction | one synthesised `outcome` record appended per copy, the real CLI run over each | the rows below |

| predictor (higher = worse) | n | AUROC | 95% CI | rate in fail | rate in pass |
|---|---|---|---|---|---|
| `Valence::negative` | 169 | 0.57 | 0.52–0.62 | 0.20 signed | 0.07 signed |
| label ≠ `neutral` | 169 | 0.50 | — | 0.00 | 0.00 |
| `Valence::negative`, `completed` runs only | 126 | 0.50 | 0.48–0.53 | 0.02 | 0.01 |
| stop cause early, `interrupted` excluded (the readout's own rule) | 168 | 0.57 | 0.52–0.62 | 0.19 | 0.05 |
| stop cause early, `interrupted` included | 168 | 0.65 | 0.59–0.71 | 0.39 | 0.08 |
| stop cause `interrupted` alone (Harbor's timeout) | 169 | 0.58 | 0.54–0.63 | 0.19 | 0.03 |
| `exhausted` (`max_turns`) | 169 | 0.55 | 0.51–0.59 | 0.14 | 0.04 |
| `ended_on_failed_call` | 169 | 0.50 | 0.48–0.51 | 0.01 | 0.01 |
| `tool_errors` | 169 | **0.40** | 0.32–0.49 | median 1 | median 3 |
| `tool_calls` | 169 | 0.41 | 0.33–0.50 | — | — |
| wall-clock seconds (Harbor's, not on any record) | 169 | **0.74** | 0.66–0.81 | median 861 s | median 180 s |

Five readings, and what each changes about what gets built next:

**1.7.1 The sign is right and the coverage is not.** Where the readout
signs a run negative it is three times as likely to have failed (20% of
failures signed against 7% of passes), so nothing it says is wrong; it
simply says nothing about 56 failures that `completed` under every ceiling
with a clean final call. That is §17's "invisible to the counters" case
measured: **59% of failures in this dataset are that case**, and restricted
to `completed` runs the readout is at chance (0.50). No counter reaches
them by construction, so a consumer that ranks by |valence| — follow-up
staging, prioritised replay — spends on ceilings and never sees the
majority. The one channel that could reach them is the appraiser (§3.10),
and this is the set to measure its marginal yield on, before any lifetime.

**1.7.2 `Interrupted` is the cheapest gain, and §3.3 is now measured.**
Harbor's wall-clock kill records as `interrupted`, and `of_session` skips
it on doctor's rule that an interrupt is an attentive owner. Here 18 of the
20 interrupted runs failed. The pair is matched: the stop-cause predictor
under the readout's own rule (every early cause it signs today, `interrupted`
left out) sits at 0.57, and the same predictor with `interrupted` read as the
ceiling it is sits at 0.65 — so the eight points are the split's own effect,
not every unsigned cause folded in. The largest single move available to
the readout, from a split the review already asked for.

**1.7.3 A tool-error count is not a failure signal, and its sign is
reversed.** Passing runs made *more* errored calls (median 3 against 1)
and more calls altogether; the failing runs are the ones that touched the
environment less. `of_session`'s refusal to count bare `tool_errors`
(agency undetermined) was argued from attribution; the data says the
count would have pointed the wrong way. The same holds for `tool_calls`
and, weakly, for turns. Nothing about *activity* predicts failure here;
only *stopping* does.

**1.7.4 The best predictor is not on the record.** Wall-clock time
discriminates at 0.74 and lives only in Harbor's `result.json`; `RunStats`
carries tokens and turns but no duration, and the `Homeostat` records load
and memory but not how long the run took. A failing run here takes nearly
five times as long as a passing one, and the record cannot say so. A
`duration_secs` on `RunStats` is a one-field change and the largest
validity gain a counter can offer.

**1.7.5 The label is degenerate on an independent verdict, not only on the
live corpus.** 169 of 169 `neutral`, reproducing §1.1 on a set where the
truth is known — every signed error is `controllable: None`, and the label
cannot leave `Neutral` without a probe. §3.1's ruling (report valence,
keep the label as an overlay) stands; the label is not an instrument.

Caveats the numbers carry. The reconstruction is not the record: fields the
loop counts at run time (`malformed_tool_args`, `tool_denied`,
`blocked_sends`, the homeostat, the boredom and escalation counters) are
left at their defaults and named so, never estimated. Four trials have no
`result.json`, ten no session file, three a session but no verdict; all are
excluded, none folded into a class. Every run was `--yes`, one prompt, no
owner, so the intervention, edit and commitment channels were structurally
empty — this measures the counter channel and nothing else, which is also
why §16's principal exists.

---

## 2. What the code does well, and should keep

Named so the changes in §3 are not read as a rewrite.

- **Derived, pointer-only, monotone.** §6.1 below: LLM-as-judge on
  trajectories has Cohen's kappa 33–41 points below what papers report,
  intrinsic self-correction degrades accuracy without an external signal, and
  a 0.94-AUROC failure critic that *intervened* cost up to 26 points. The
  design's refusals — no self-report, no affect in the prompt, sensors before
  consumers — are each the conclusion of a paper published after the design
  was written.
- **Step appraisal and boredom are the healthiest part.** Model-free,
  in-run, and `step::looks_like_verification` with no verify-shaped call is
  Terminal-Bench's own "reasoning–action mismatch" failure category. The
  literature's other validated model-free counters (§6.1) extend this family
  rather than replacing it.
- **`of_session` is per session and `live` is per run**, and the reasoning
  that keeps them apart (an intervention has a message index and no run
  index) is correct and was measured at 5.9× before it was caught.
- **`Followup` exclusion was right for the raw channel.** An ordinary second
  question read as a −1.0 would have dominated. The fix in §3.4 reads the
  judged subset, not the raw one.

---

## 3. What to change, in order

Ordered by what each unblocks per unit of work. The first two are the ones
that make everything after them measurable.

### 3.1 Report valence; keep the discrete label as an overlay

Add a dimensional readout beside `Affect`: the signed magnitude the record
already carries (the most-negative error's sign and magnitude, the count of
negatives and positives, and whether any was visible), returned from `live`
and `of_session` and shown on the three surfaces and in `--json`. The discrete
label stays as the second line, derived exactly as now, and fires when its
dimensions are filled. This is the "average over the dimensions that are
*present*" rule Marinier et al. chose after rejecting the product, and EMA's
shape — one relevance gate, then a label from desirability and likelihood
alone. It also dissolves `live`'s compacted-run problem: with a dimensional
readout, dropping the interventions loses one term instead of flipping a
label, so the `Neutral`-outright guard can become "valence with a
`partial` flag". `affect_of` itself does not change.

*Argues with* §6's "discrete because derivable and testable" — both are
derivable; the corpus says only one of them is informative today.

> **Built 2026-09-02** (`feat/appraisal-record`, merged as #140 at
> `15c628d`): `appraisal::Valence`
> (positive and negative sums kept apart, counts, `visible`, `partial`) and
> `Readout { label, valence }`, with `live_readout` beside `live`. The owner
> ruled the rendering per surface: a number on the TUI badge and in a Slack
> thread context line, a two-sided bar on the web chip, voice unchanged.
> `sessions appraise` prints and emits the summed valence. Over the live
> store the same day: 18 of 143 sessions signed, `+12.0 −19.5` across them,
> where the label said `neutral` on all 143. A compacted run's label stays
> `Neutral` and its valence is computed from the counters with `partial`
> set. Two things moved with it: a ceiling reads as `Agency::Owner` (the
> owner's own limit) rather than `World`, so it no longer labels `Anger` —
> the closure follow-up gate reads `Appraisal::cut_short` beside the label
> to keep staging the residue of a ceiling-cut `done`; and the free
> readout's label range is `Neutral` alone.

### 3.2 Give a session a kind, and exclude tests from the instrument

Put a `kind` on the session `meta` record (`chat`, `tui`, `web`, `voice`,
`task`, `trigger`, `frontdoor`, `slack`, `eval`, `test`), written by the
front-end that opened the session, and add `--kind` to `sessions appraise`,
`sessions health` and `runlog`. Smoke tests run by a session developing mecha
should write `test`, or run with the `--no-session` flag `mecha run` and
`mecha tui` already take.
Until this lands, every corpus number about the appraisal is a number about
the harness's own test runs, and the rung 7 measurement that decided the
build order was taken over the same mix.

> **Built 2026-09-02**: `session::SessionKind` on the `meta` record,
> written by every front-end (`run`, `chat`, `tui`, `web`, `voice`, `task`,
> `trigger`, `frontdoor`, `mail`, `slack`), lenient on read; the one
> override is `MECHA_SESSION_KIND=test`, applied in `Session::create` and
> able only to narrow to `test`. `runlog::Scan` gained `kind` and
> `include_tests`, `Scan::admits` is the one admission every corpus reader
> shares, and `sessions list|health|appraise` take `--kind` and
> `--include-tests`; a `test` row is out of every readout unless asked for
> by name, and a row from before the field matches no `--kind` filter.
> Existing rows stay unknown: the 46-of-143 figure above is what it is, and
> the next honest measurement needs sessions recorded after this landed.

### 3.3 Split `Interrupted` into what ended the run

`StopCause::Parked` for a question put to the owner (`questions.rs`'s cancel
is the one site), `Cancelled` for a person or a shutdown. A closed enum in
an append-only store is a wire format, so old rows keep `Interrupted` and
readers treat it as unknown-which. Then two new deterministic channels
become honest: **cancelled and re-prompted in the same session** is an
intervention (`Channel::Intervention`, `Agency::Owner`, `Cite::Turn` at the
re-prompt), and **cancelled and never resumed** is the same with no
aftermath — the abandonment signal the dialogue-feedback literature ranks
highest (§6.2). A park is the mechanism working and stays out, as now.

### 3.4 Read the judged followups

`of_session` takes a slice of reflections for the session and emits an
intervention error for each followup-triggered one, `controllable` left
`None`. This is a model's judgement, but a gated and recorded one — the same
standing `apply_probe` gives the replay's verdict — and it is the owner's
own next turn that was judged, which is injection-safe under the provenance
rule. Twenty-two such rows exist today against thirteen raw interventions.

> **Built 2026-09-02** (`feat/appraisal-phase-b`, merged as #141 at
> `49166e3`): `of_session` reads
> `reflections.jsonl` through `SessionRecords::reflexions` and signs a
> follow-up-triggered reflection for the session as an `Intervention`
> error, `Owner`, `controllable` unfilled, cite `Cite::Reflexion(id)` —
> only where `Reflexion::provenance()` is clean — stricter than the
> learning loop's `learnable()`, which carries a triage-domain exemption
> this deliberately does not (the owner's ruling, 2026-09-02: a
> wider owner-turns clause admitted nothing the live path writes, since a
> tainted session's reflection is recorded clean by construction), never
> a dropped one, and never a steer or denial (the raw channel already has
> those).

### 3.5 Replace the guilt sensor's level with the run's delta

`anticipated_guilt` should be computed from `backlog_delta` (what this run
did to the owner's queue) with the standing level as a modifier, not the
other way round — and the delta is signed, so a run that cleared a question
or released a draft is the first *positive* commitment channel, read from a
store only the harness writes. §7.4's safety argument is unchanged: a delta
on `OutboxStore` cannot be manufactured by a sentence in a fetched page.

> **Built 2026-09-02**: `guilt::with_delta(level, net_delta, waiting_before)`
> — the level is scaled down by the *share* of what was waiting that this
> run cleared: a run that cleared everything it inherited reads as no guilt,
> one that cleared three of forty reads nearly the level it inherited (the
> first cut divided by the constant `COUNT_HALF_AT` and pinned three cleared
> to zero from any backlog — found on review). Both numbers come from the
> three owner-facing stores (`BacklogDelta::owner_facing_net`, `guilt::
> waiting`), never the five-store `net`, and `guilt::with_backlogs` derives
> them from one pair of reads. Both read *clearance*, not the fall:
> `Depth::given_up` counts the rejected drafts, abandoned questions and
> closed-unsent requests, and `BacklogDelta::owner_facing_cleared` takes
> the rise in those off the fall before either the relief or the
> commitment arm's positive is credited — a queue the owner shortened by
> giving up had signed `+0.5` in the same channel the question and request
> arms signed `-0.5` for the same act (found on review). The fold lands in its own field,
> `Homeostat::guilt_after_relief`, so the level's corpus mean stays one
> quantity across old and new rows. A run that *added* to the queue reads
> the level it inherited,
> because staging is its job and the first cut's "added three reads as
> maximal" was exactly the reading `Homeostat::finish` refuses by name
> (found on review). `Homeostat::finish` computes the delta first and folds
> the level through it, and `RunStats::merge` sums `backlog_delta` across a
> session's runs where it kept the first run's before — a session that
> parks a question is resumed by construction, and the resume is where the
> clearing happens. And the delta is a channel:
> a session whose `backlog_delta` net is negative signs `+0.5`, `Own`,
> `Channel::Commitment`, cite `Setpoint("backlog_delta")`; adding to the
> queue signs nothing, because staging is a trigger's job.

### 3.6 Build the two positive channels §5.2 named and nobody built

Both are one join each over stores that exist: **a question answered and the
resumed run finishing clean** (`QuestionStore` status `answered` joined to
the session's next outcome), and **a front-door request closed by a draft
sent unchanged** (`Frontdoor` state joined to the outbox item it produced).
Three answered questions and five closed requests exist today; both channels
are owner-authored by construction. The third — a trigger's artifact acted on
— has no producer: nothing records whether a briefing was read, and the
morning trigger has run 14 times into that silence. That one needs a surface
change (a read receipt on the briefing page or the Slack post) before it can
be a channel, and is named here rather than proposed.

> **Built 2026-09-02**, as `Channel::Commitment`: a question this session
> parked and the owner answered, with the session then finishing of its own
> accord, signs `+0.5`/`Own` (`Cite::Question`); an abandoned one signs
> `-0.5`/`Owner`. A front-door request this session triaged that the owner
> closed with none of its drafts sent signs `-0.5`/`Owner`
> (`Cite::Request(seq)`); one answered by a sent draft is already the draft
> channel's positive and is not counted twice. `sessions appraise` and the
> closure appraisal read all three stores best-effort and say which could
> not be read; `distill` and the live readout read drafts only. The trigger
> read receipt is still unbuilt. Re-read over the live store with phases A
> and B together, after review: **27 of 144 sessions signed, `+16.5 −34.5`
> across them** (intervention 26, edit 22, commitment 4, counter 2), label
> `neutral` on all 144 — the judged follow-ups doubled the intervention
> channel.

### 3.7 Pre-register an expectation, and score it

Every appraisal model reviewed makes *expectedness* the intensity axis, and
the OCC prospect quartet (satisfaction, disappointment, relief,
fears-confirmed) exists only because a prediction was made before the
outcome. mecha already makes harness-authored predictions in every run —
`forecast()` on the `todo` result (will the plan fit), `pressure.rs` (the
next request's size), `step::escalation_candidate`'s span-versus-siblings —
and scores none of them. A `Channel::Expectation` error with
`Cite::Counter` naming the prediction, signed by the residual, costs no model
call and is the first channel that can say a run went *better* than
expected. A plan's step count against its actual tool calls is the cheapest
first predictor. BAGEN (§6.4) finds model-authored budget predictions are
systematically optimistic with interval coverage capped at 47%; the harness's
own arithmetic predictions have no such bias, and the model's, if ever
recorded, are hearsay whose *residual* is still a signed, endogenous signal
per model — a calibration series `diagnose` could read.

> **Record built 2026-09-02**, to the spec agreed with the audit lane
> (`AUDIT-RESEARCH.md` §3.11): `TodoItem::{expect, check, expect_calls}`,
> strict from the model and lenient from a record, rendered under the step
> and round-tripped through the carried block; the check frozen on
> completion with a tamper echo; `step::CHECK_TRACE`, `Work::{checks_declared,
> checks_passed}`, `Finding::CheckFailed`; `RunStats::{checks_declared,
> checks_passed}`; a failed check signed `-1.0`/`Own`/`checks_passed` in
> `of_session` and counted as `cut_short`; and `learning::Trigger::Mismatch`
> as the wire word. **Not built here**: the loop running the check and the
> planner's ask (the audit lane's), the `expect_calls` residual in
> `escalation_candidate` (theirs too), the reflection that fires on
> `Mismatch`, and the tamper count folded into `RunStats` (the loop would
> have to ask the plan tool by name). The corpus for scoring starts when the
> ask lands and a model writes its first `expect`.

### 3.8 Extend the model-free counters step appraisal reads

Three counters the trajectory literature has validated (§6.1) and mecha
does not keep: **same-region re-edit count** (coherence collapse — 60–69% of
capable-model failures reach and edit the right function, then thrash it),
**acting after declaring done** (unaware of termination), and **a test or
verify claim with no non-zero exit code read** (the `looks_like_verification`
rule, extended to `shell`). Each is `Agency::Own`, `Channel::Counter`, and
each is a *kind* that can recur within a session, which is what
`Frustration`'s repetition test has never had a producer for.

### 3.9 Spend the replay budget on the sign

§8's prioritised replay is the consumer that makes the whole record matter,
and it is unbuilt because it was to key off a degenerate label. With §3.1 it
keys off |valence| — sessions with the largest signed error first, the
uniform holdout `sample.rs` already provides as the control. Affect stays a
priority function and never an objective one, exactly as §8.3 rules.

### 3.10 Measure the appraiser once at scale, then keep it or retire it

The quarantined appraiser sees only `AppraiserEvidence`, which is built from
the already-computed `Appraisal` — so by construction it can add an error
only where the numbers already imply one. Every smoke test returned "nothing
further". Run `mecha sessions appraise --appraise --max-appraisals 143` once
over the store; if the yield is near zero it is a paid no-op and should be
retired, or given one more structured input (the plan's step list with
statuses, the model's own prior output, on step escalation's precedent).

### 3.11 Two small corrections

- A ceiling (`MaxTurns`, `OutputTokenBudget`, `CostBudget`) is labelled
  `Anger` via `Agency::World`, and it is the only non-neutral label a surface
  shows today. Roseman's split puts low control potential at
  distress/sadness and high at frustration; "the owner's own limit was
  reached" is neither anger nor world-agency. Either label it `Owner` (the
  owner set the number) or give ceilings their own word.
- `controllable` can be filled deterministically in one narrow case: a
  denial where the same turn's tool surface held a read-only alternative
  the model did not take. Everything wider stays the probe's.

> **Built 2026-09-02**: the ceiling relabel, as above. The denial
> controllability heuristic is not built; it waits for the probe corpus to
> say whether it agrees with the replay on the cases both can reach.

---

## 4. Deliberately not proposed

- **A model reading the transcript and saying how it went.** §6.1: judges
  on trajectories are position-biased, self-preferring and 33–41 kappa
  points less reliable than reported; failure *attribution* by the best
  automated method finds the decisive step 14% of the time.
- **Mood-congruent anything.** EMA adds mood to each candidate emotion's
  intensity before choosing; §15 rules that out here as the rumination loop,
  and the evidence that a model fails more with its own errors in context
  supports keeping it out.
- **Affect steering a run.** The early-abort literature is unanimous:
  predict and log; a critic that intervenes on a 0.94 AUROC lost up to 26
  points on high-success tasks.
- **A store.** Every change in §3 is a pure function of records that exist;
  the derivation still replays over the whole corpus, which is the property
  the no-store rule protects.

---

## 5. What the appraisal literature says, applied

Summarised from the research pass; primary links follow each claim.

**Every model has one gate and then labels from few variables.** EMA builds
an appraisal frame only where |utility| exceeds a fixed constant, then
derives hope/joy/fear/distress from desirability and likelihood alone
([Gratch & Marsella 2004](https://people.ict.usc.edu/~gratch/GratchMarsellaCOGSYS04.pdf)).
Scherer's relevance check is explicitly a gate before the implication,
coping and norm checks. OCC separates *type* from *intensity*: intensity is
potential minus threshold, and thresholds move with mood
([Steunebrink et al. 2008](https://people.idsia.ch/~steunebrink/Publications/ECAI2008_0337.pdf)).
Marinier, Laird and Lewis rejected a product over dimensions because "if any
dimension has a zero value, the intensity will be zero regardless of the
other values", and averaged over present dimensions scaled by a surprise term
([Marinier et al. 2009](https://public.websites.umich.edu/~rickl/pubs/marinier-laird-lewis-2008-cogsys.pdf)).
mecha's `label_of` is the rejected product in a different costume.

**Expectedness is the intensity axis everywhere.** PEACTIDM writes a
prediction at every Intend step and reads discrepancy at Comprehend;
confirmed predictions decay to zero intensity ("we might now call the
emotion boredom") and disconfirmed ones persist. CHI 2024 computes
suddenness from transition frequency, relevance from |TD error| and
conduciveness from clipped TD error against human ratings
([Zhang et al. 2024](https://jyx.jyu.fi/bitstream/handle/123456789/94912/3613904.3641908.pdf)).
The cheap dimensions from an agent's own records are novelty, relevance,
conduciveness and control; the expensive ones are norm compatibility and
causal motive, which nobody computes from logs
([Moerland, Broekens & Jonker 2018](https://arxiv.org/pdf/1705.05172)).

**Controllability has a cheap form.** EMA's test is whether the plan holds
an action that could re-establish the goal — a "white knight" — computed
from the plan, not from a replay. mecha's replay is the strict form and
stays ground truth; §3.11's denial case is the white-knight test on a
transcript.

**Mood is a two-rate integrator, separate from emotion.** Marinier moves
mood 10% toward the current emotion per cycle and decays it 1%; WASABI runs
valence and mood as two coupled springs with different return rates
([Becker-Asano & Wachsmuth 2010](https://cs.uwaterloo.ca/~jhoey/teaching/cs886-affect/papers/WASABIAAMAS2010.pdf)).
§6.1's decision to keep sadness and boredom as recomputed moods is this
shape; nothing in the tree computes either yet, and `Homeostat` has no
field for one.

**Coping has no controlled performance result.** EMA's coping strategies
are validated against a clinical inventory, not task success
([Gratch & Marsella 2005](https://link.springer.com/article/10.1007/s10458-005-1081-1));
the RL results (Marinier & Laird 2008; Sequeira et al. 2014) are reward
shaping in small environments. No 2023–26 LLM-agent paper derives affect
from run records and feeds it into control; the nearest are affect-free
telemetry detectors ([arXiv 2608.02464](https://arxiv.org/abs/2608.02464v1)).
mecha is, as far as this pass could find, alone in this shape — which is a
reason to measure before consuming, not a reason to stop.

**Unverified by the pass:** Sequeira's numeric results, FAtiMA's
threshold/decay API fields, Roseman's exact dimension list (secondary
source), and Scherer's sub-checks (reconstructed from Marinier's Table 3).

---

## 6. What the harness literature says, applied

**6.1 Grading without a human label.** Verifiable, model-free checks
dominate wherever they exist (Terminal-Bench hidden tests,
[2601.11868](https://arxiv.org/abs/2601.11868); SWE-Gym's trained verifier
recovers only ~74% of Pass@16 headroom,
[2412.21139](https://arxiv.org/html/2412.21139)). LLM judges on trajectories:
kappa 33–41 points below reported, position bias, rankings shifting 14
places across benchmarks ([2606.19544](https://arxiv.org/pdf/2606.19544));
best automated failure attribution finds the decisive step 14.2% of the time
([Who&When, ICML 2025](https://arxiv.org/abs/2505.00212)). Intrinsic
self-correction degrades accuracy without external feedback
([2310.01798](https://arxiv.org/pdf/2310.01798);
[TACL survey](https://direct.mit.edu/tacl/article/doi/10.1162/tacl_a_00713/125177)).
Validated model-free trajectory signals: failed runs are longer with higher
variance *within* a model ([2511.00197](https://arxiv.org/abs/2511.00197))
but length does not transfer across models
([2601.11868](https://arxiv.org/pdf/2601.11868) §G.1); coherence collapse —
right function reached, then thrashed — in 60–69% of capable-model failures
([2603.24631](https://arxiv.org/pdf/2603.24631)); Terminal-Bench's failure
taxonomy (step repetition, unaware of termination, reasoning–action
mismatch, premature termination, weak verification). A 0.94-AUROC critic
that intervened cost 0 to −26 points
([2602.03338](https://arxiv.org/pdf/2602.03338)).

**6.2 Implicit owner feedback.** Copilot: acceptance correlates ρ=0.24 with
perceived productivity, persistence at 30–600 s slightly *less*
([2205.06537](https://arxiv.org/abs/2205.06537)); Cursor's "survival share"
of accepted lines at 60 minutes is ~80% ([cursor.com/insights](https://cursor.com/insights)),
so the informative mass is the ~20% edited or reverted — mecha's
`SentUnchanged`/`SentEdited` split is the same label with the right
polarity. Dialogue: the next user turn carries feedback far more often than
not, negative is common and positive rare, and a detector reaches 81% on the
easy setting and 42% on the dense one — "noisy as a learning signal"
([2507.23158](https://arxiv.org/pdf/2507.23158)); ReSpect decodes
rephrase/frustration/pivot and lifted completion 31% → 82%
([2410.13852](https://arxiv.org/html/2410.13852)); 91.5% of coding-agent
failure resolutions required explicit user correction, and inaccurate
self-reporting *rose* over time while failures fell
([2605.29442](https://arxiv.org/html/2605.29442v1)).

**6.3 Goal drift and plan decay.** Drift grows with context and is driven
by prefix pattern-matching ([2505.02709](https://arxiv.org/abs/2505.02709));
a plan's hidden-state signal decays 4–12× one step after it is written and
periodic reminders reduce violations ([2606.22953](https://arxiv.org/pdf/2606.22953);
[2604.12147](https://arxiv.org/html/2604.12147v2)) — `carried_state`
rendering the goal above the list is the shipped form of that reminder.
Decisive evidence of failure occupies 5–11% of turns and appears 59–84% of
the way through ([2606.05414](https://arxiv.org/pdf/2606.05414)), which is
an argument for run-end appraisal over per-turn.

**6.4 Calibrating the harness's own predictions.** BAGEN: agents asked to
bound their remaining budget are consistently over-optimistic, coverage
capped at 47% ([2606.00198](https://arxiv.org/pdf/2606.00198)); RLCR adds a
Brier term with no accuracy loss ([2507.16806](https://arxiv.org/pdf/2507.16806)).
Predicted-vs-observed difficulty residuals expose contaminated and
infeasible tasks ([2608.05797](https://arxiv.org/html/2608.05797)). This is
§3.7's evidence base: the residual is the signal, and its bias is per model.

**Unverified by the pass:** Terminal-Bench's per-category failure
percentages (figure only), the EMNLP feedback study's population-level rate,
and several 2026 papers read as abstracts.

---

## 7. Open, and named so it is not rediscovered

- **What valence a surface shows on a run with only positive errors.** §3.1
  makes a positive-only run readable for the first time; whether the badge
  shows it (a gauge always on trains people to stop seeing it — §16's own
  open item) is a UI question this file does not settle.
- **Whether a model-authored prediction should ever be recorded.** §3.7
  scores harness arithmetic only. A model asked to predict its own step
  count produces a hearsay number whose residual is still informative;
  whether that residual belongs in the appraisal or only in `diagnose` is
  open.
- **The mood integrator.** Designed in §6.1, absent in code, and nothing
  reads one. Build it when a consumer needs a trend; do not persist it.
- **Review-queue salience in pkg** still needs the other repository to read
  `meta.affect`, and with §3.1 it should read valence instead.
- **Charter sensors.** Ruled in on 2026-09-02 and designed at
  `GOAL-SYSTEM-DESIGN.md` §11.1 with seven containments; unbuilt. It is the
  producer for `GoalError.goal` on an ordinary run, and the by-id
  attribution that closes the queue-delta arm's accepted residual — a
  global before/after diff credits a run for what the owner cleared by
  hand mid-run, as `live_readout` discloses.
