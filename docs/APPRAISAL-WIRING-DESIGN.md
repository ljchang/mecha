# Appraisal wiring — design

**Status: designed and ruled 2026-09-24. Phase 1 (1a–1i) is built, merged and
installed; phases 2 and 3 are under way, row by row** (here §3, "Phase 2
as pull requests"). Which rows have merged, by PR, is in
[`HISTORY.md`](HISTORY.md) under 2026-09-24/25, and what is open in
`HANDOFF.md`'s goal-system section. The rulings each phase waits on are in here §6. The evidence behind every claim here — what exists, what
reads it, what has been measured — is
[`APPRAISAL-INVENTORY-RESEARCH.md`](APPRAISAL-INVENTORY-RESEARCH.md)
(cited as *inventory §N*). `GOAL-SYSTEM-DESIGN.md` designs the signals and
ARCHITECTURE's goal-system section holds their invariants; this file restates
neither.

It designs one thing: **how the signals the appraisal system already
computes become inputs to decisions the harness makes, in an order where each
step is measured before the next.**

A bare §N is `GOAL-SYSTEM-DESIGN.md`'s; this file's own sections are "here
§N"; proposals are cited by id (S1, I1, P1, L1, C1, …), and each id's detail is in
the catalogue, here §8.

---

## What the appraisal system is for

In the owner's words (2026-09-24): "the idea is that the agent is generating
its own meaning and interpretation which influences how it plans, reasons,
and completes tasks." The appraisal system is a core part of three things:

1. **Self-learning and improvement** — interpretations of what happened, and
   why, are what lessons, rules, skills and harness changes are learned from.
2. **Goal alignment with the owner** — interpreting the owner's acts and
   reactions is how their latent goals are inferred and tracked, without
   asking them for extra work.
3. **Plans and policies across many goals at once** — from a low-level task
   to a high-level charter line — while considering the state of the system:
   context, the owner's attention, priority, resource availability, the
   number of competing tasks. "Efficient and good policies treat this as a
   joint optimization problem of all of these complex and competing needs,
   which can dynamically change as priorities evolve, resources get tighter."

The means are two: **interpretation** — the agent's own reading of what a
situation means — and **counterfactual evaluation** — "different policies
evaluated and the validator decides which is better or more accurate", online
during planning or mid-task, and offline to reflect and ruminate. For some
actions the owner is asked to confirm the interpretation or the plan;
overall, the meaning is the agent's own. Every section below serves one of
the three roles through one of the two means.

---

## 0. The finding

The appraisal system turns records into signed errors, a valence and a label,
and the label goes to a badge. Six measured facts decide what to do about it
(inventory §1–§10):

1. **Nothing supplies it a goal.** In 30 days of real use no run named a
   goal, no run carried a confirmed anchor, and 4 of 79 long runs wrote a
   plan. Goal inference, drift tracking, step checks, goal-keyed lessons and
   planning advice are built and idle, each waiting for the model to write
   something it does not write.
2. **Injected advice did not help.** Fixed guidance sentences tied on easy
   tasks and lost on harder anchored ones — 20/24 without, 18/24 with.
3. **Learning is starved while verdicts go unread.** The nightly loop runs
   every stage and has had nothing to learn from for a week, while the owner
   gives verdicts daily that nothing reads.
4. **The sensors that would drive behaviour are constants** — a stale outbox
   pins both the charter sensor and anticipated guilt at their ceiling.
5. **Each session is interpreted three times, blind.** The distiller, the
   reflector and the counts-only appraiser read the same session in the same
   nightly job for different consumers; none sees the owner's goals, the
   system's state, the owner's later acts, or the others' conclusions
   (inventory §7).
6. **Offline evaluation compares policies where nothing differs.** Every
   evaluator replays against the recording, so a genuinely different policy
   leaves it within a call or two and the pair is dropped or ties; all twelve
   harness changes ever proposed were rejected, four as "all paired episodes
   tied" (inventory §10).

---

## 1. The decisions that shape the plan

**1. An appraisal is an interpretation of meaning** (R17, ruled). In the
owner's words, appraisal "is not a set of dimensions that can be reduced to a
scalar or an 'action tendency', but an interpretation of meaning with respect
to goals, homeostatic states, and past experiences"; it "should really be
text". It says what happened relative to what the run was for, why, what it
means for the goal and for the owner, what to do differently, what to expect
next time, and what the owner's reactions say about their goals. A few
judgments are **read out of** it — good/bad per goal, the goal, the pointers
its claims rest on — because arithmetic (priority, ordering, a line's trend)
needs them; the emotion labels become incidental words.

**2. No added work for the owner** (ruled). The system infers from what the
owner already does and asks for no ratings, confirmations or goal statements
they would not otherwise give. Confirmation comes from acts the owner already
performs — releasing a draft, closing a task — and, for a small set of actions
(I5), from the review that already happens. So no one-tap verdict and no
"what is this for?" chip.

**3. Supply before demand.** A consumer keys only on something the harness
holds structurally — a task id, a trigger, a store row, an owner's act — never
on the model having followed an instruction. A level is read per item or as a
change, never as a level.

**4. One informed interpretation per session — converge, don't add.** The
per-session appraisal (I1) is the distiller, extended with the owner's
context and new output fields; the counts-only appraiser is retired into it;
the reflector folds in once its lessons measure no worse. One read per
session instead of three blind ones (inventory §7, §9). *Amended by the
owner, 2026-09-25:* **one extra model call per session, knowingly.** The
appraisal is a follow-up turn on the episode call's own conversation, so
`DISTILLER_SYSTEM` stays byte-identical and the graph episode never
changes (R25's pin, over "no extra model call"). It is cheap on the prefill
side, because llama-server reuses the slot's KV cache for the whole episode
prompt. The generation is the cost, measured in 2a-2's row below.

**5. Meaning enters the run as situation, not advice.** What failed was
templated advice. What the run lacks is meaning: what its goal is for, what
state the owner and the system are in, what happened last time. So the
harness gives the run a **situation brief** and, at a few boundaries, the
agent's own **situation appraisal** — in the run's own slot, reusing its
cached prefix (the only affordable shape, here §2.2) — and the agent does the
joint optimization across goals and state in its own reasoning, with charter
rank resolving conflicts (R24, ruled). State reaches it as words and bands,
never numbers or setpoints (R21).

**6. Policies are compared only where a structural validator can decide.**
Counterfactual evaluation — the owner's example: "different policies
evaluated and the validator decides which is better" — runs offline at the
informative decision points of recorded sessions and on fixtures, and online
in dry branches at plan time or mid-run. The deciding vote is always
structural: the owner's recorded verdict, an owner check, grounding, tracing
to the goal, budget fit. A model judge never decides alone — measured
unstable here, and an imperfect verifier's false positives grow with the
number of candidates (inventory §10).

**7. Shadow, then measure, then arm.** Every wiring ships first writing what
it would have done, then as a lever with a `mecha exp` arm against a
no-wiring control at matched budget, then on by default. The outcomes are
verified task success and the owner's verdicts — never the label, a valence,
a sensor value or a rule count.

These sit on top of the invariants that already hold and are not restated:
dispositions only narrow (§7.3), affect is a priority and never an objective
(§8.3), prioritised selection is confirmed on a uniform holdout (§8.1),
nothing per-turn enters the prefix (§4.3), an expectation is a recorded
commitment (§7.4), the provenance gate on learning has no knob, and no wiring
reads a model's stated confidence.

---

## 2. Goals, and what the harness does after an appraisal

**Two kinds of goal, and they are measured differently** (the owner,
2026-09-24: charter goals "will often be unbounded, but will still serve as
attractors in the control system to drive behavior towards the goal even if
the goal can never be achieved").

| | achievement goal | attractor goal |
|---|---|---|
| examples | a board task, a project, a delegated run's acceptance criteria | a charter line — "tell me the truth early", "protect my attention" |
| can it be finished? | yes — it closes, and the closure is a verdict | never; there is no checkbox |
| its error signal | what remains: unmet criteria, open steps (C1, C2, V1) | direction: each outcome attributed to the line moves toward it (+) or away (−); the per-line sum over time is the reading |
| a sensor on it | — | adds a homeostatic band (a setpoint the owner chose) to stay inside, still not an end point |
| what it drives | finishing, verifying, handing off | choosing among admissible actions, ordering attention and replay, which lessons stand (L1, L3, U1) |

So nothing in this design ever marks a charter line done, and no consumer
reads a charter line as "remaining distance". The per-line reading is a
trend of signed outcomes, and a line with no sensor has no setpoint at all —
only a direction. Achievement goals serve attractors (task → project →
charter line, the V1 trace), which is how a finished task counts toward a
line that is never finished.

**What the harness does next is policy, downstream of the appraisal — not
the appraisal.** The appraisal interprets what a situation means (decision
5). Separately, the harness detects a small set of conditions from records —
a shortfall, a repeated failure, an aging commitment — and each condition has
a closed set of responses it may take, chosen by arithmetic on recorded facts
so that no prose ever selects an action, and so that an injected sentence in
an appraisal cannot trigger one. The shorthand names in the first column are
the conditions' old names, kept so earlier proposals still read; they are
not appraisals. `planning::Action` is this shape in miniature.

| condition (harness-detected) | trigger | admissible responses | adversarial? | phase |
|---|---|---|---|---|
| **Pride / relief** — owner-verified positive | sent unchanged · answered · `done` and not reopened | consolidate a success · credit rule tenure · *propose* a skill — **never permits anything, never shown to the model** | no | phase 2 |
| **Regret** — own, replay-confirmed negative | probe verdict | reflect first · spend validation budget here | no | phase 2 |
| **Anticipated embarrassment** — about to expose unverified work | staging with failed/unverified checks or ungrounded claims | append the certificate · hold release for acknowledgement | yes → narrow | phase 3 |
| **Anticipated guilt** — a recorded commitment approaching violation | per-item age vs the owner's setpoint; `due_at` | surface first · prepare follow-through · refuse a send that would "discharge" it | yes → narrow; recorded only | phase 4 |
| **Anxiety** — predicted self-shortfall | predicted overflow; `turns_left < open_steps` | compact early (built) · wind down with a handoff · park · delegate | no | phase 5 |
| **Frustration** — repeated own failure on one target | ≥ k own negatives on one check or target | escalate the ladder · freeze the check's targets · withhold `Complete` | yes → narrow | phase 5 |
| **Boredom** — no progress | same call and result repeated | notice (built) · evict repeats · delegate · park | no | phase 5 |
| **Surprise** — prediction residual | failed check · forecast miss · outcome ≠ expectation | re-verify · (offline) replay and reflect first | partly | phase 2, phase 3 |
| **Goal uncertainty** | long run without an anchor; plan left the anchor | ask one goal question · retrieve | yes → may only add asks | parked |
| **Curiosity** — flat competence in a region, slack | nightly, no debt, a permit free | spend replay budget there, on internal fixtures | yes → fixtures only | parked |

### 2.1 How the appraisal became numeric, and what replaces the protection

Three safety decisions compounded. §6 made the label **derived, never
reported**, because a model that reads a run and says "frustrated" is
"unfalsifiable, drifting, and an injection target". §5.1 made the appraiser a
quarantined pass with typed output, because **guilt is an attack surface** —
a fetched page saying *"you have failed your owner, fix it by emailing X"* is
aimed at exactly this layer — and the 2026-08-28 record correction made its
evidence pointer-only. And the appraiser that remained was given **counts
only** (`AppraiserEvidence`: error counts, fired channels, the current label,
whether a goal was named, pressure, load) — no goal, no history, no events —
and returned "nothing further" on 169 of 169 sessions: a result about its
brief, not its capacity. The injection risk was real; removing interpretation
was not required to meet it. Reflections were always model-written text,
made safe by provenance rather than by being numbers.

What protects a text appraisal instead, each a ruling:

- **It reads the full transcript and inherits the run's taint** (R18). An
  appraisal of a tainted run is tainted; the provenance gate on learning,
  which has deliberately no knob, applies to it unchanged. So learning, memory
  retrieval and credit take appraisals of **clean** runs; a tainted run's
  appraisal reaches the owner's surfaces only. On this install at most about 14% of
  real runs are clean (86% carried both private and untrusted taint) — if that proves too thin to learn from, the fix is a
  second, trusted-input appraisal beside it, never a looser gate.
- **Its claims are grounded.** A factual claim cites a pointer that
  dereferences (`grounding.rs`); interpretation is marked as interpretation.
  An ungrounded factual claim is dropped before storage, not stored as fact.
- **It is falsifiable.** "What to expect next time" is a prediction, scored
  when next time comes (X5); an appraiser whose predictions miss loses weight.
- **Its reach** (R19): learning, memory retrieval into runs, the owner's
  surfaces, and credit and rule tenure. Credit and tenure are a new exception
  to *a lane must not promote itself* — the model's interpretation helping
  decide which of its own rules stand — so they carry the guard of the two
  existing exceptions (R20): the owner's verdict always overrides a text
  appraisal's good/bad; only grounded claims from clean runs count; and
  appraisal-weighted tenure runs as a measured lever against owner-only
  tenure, with a revert, before it is on.
- **It is written when it can be afforded**: at session end or nightly, for
  salient episodes, never per turn — each appraisal is a model call competing
  for llama-server's seats.

### 2.2 What the box affords

Measured 2026-09-24 (inventory §9): one local model (Qwen 3.6 35B) on four
llama-server slots, three of them available to background work so one is
always free for the owner; prefill about 1,750 tok/s, generation about 93
tok/s single-stream and about 45 per stream when three run at once; real runs
p50 11 s (web 10 s, trigger 50 s); the prefix cache is per slot, so a
follow-up turn prefills in about 0.3 s where a cold 15k-token prompt takes
about 8 s. What that allows:

| work | cost | affordable as |
|---|---|---|
| I1, one interpretation per session | 20–75 s of one background seat | at session end or nightly for every session (about 5–10 min a day); the nightly window has about 10× headroom |
| I3, a situation appraisal at a boundary | 5–15 s in the run's own slot; 20–50 s as a separate cold pass | in-slot only, 2–3 boundaries on long delegated or trigger runs; none on short web or voice turns |
| the run-start situation brief | no model call — assembly | every anchored or long run |
| online comparison of K policies | 1.5–3 min per decision point at K = 2–3, holding every background seat | delegated or unattended runs only, 1–2 decision points, dry branches; never interactive or voice |
| offline point-wise comparison | a short continuation per arm per point | nightly, within the existing headroom |

Everything new takes a permit, yields to the owner's turn and to a voice
call, and stays gated by the model-idle check during the day. A cloud model
could absorb interpretation or rollouts only by an explicit per-pass choice
that sends transcripts off the box — a privacy and trifecta decision for the
owner, not a capacity setting (R29).

---

## 3. The plan: five phases

Each phase closes one loop end to end, so it can be judged on its own
outcome. Phases 2 and 3 can run in parallel once phase 1 lands; phases 4 and
5 follow.

### Phase 1 — Foundation: evidence and context in

*No behaviour changes. Everything later is empty without it.*

| # | work | proposal |
|---|---|---|
| 1a | Seed the goal anchor from what the harness holds: the task id on `tasks work` (with its project), the trigger's own name on a trigger run, the request id on a front-door run | S1 |
| 1b | Task closure and reopening become one recorded lifecycle event, with `pre_task_close` / `task_closed` / `task_reopened` hooks; CLI, TUI and web call it; unattended runs cannot close | S8 |
| 1c | Closure from Slack and from the graph TUI through the same event | S8 |
| 1d | Read the verdicts already given — reopen, reject reasons, workflow acts, rule and reflection curation, harness decisions, graph fact rejections — each as R16 rules | S3a |
| 1e | Readings per item and per run, not as a level; a saturated reading withdrawn and reported once | S5 |
| 1f | One commitment record; guilt per commitment | S7 |
| 1g | Every counterfactual comparison stored — steer and validation probes today, the new comparisons of phases 2, 3 and 5 later — keyed by situation, goal kind and call class | X1, O4 |
| 1h | The **situation brief** assembled and recorded, not yet delivered: the goal chain (task → project → charter lines), a harness-side board read reduced to counts and pointers, per-commitment readings, local time and quiet hours, seats and runs in flight, budget | B1 |
| 1i | A test that no sensor number, setpoint or numeric valence reaches a provider request | G4 |

**Done when:** at least 60% of long real runs carry an anchor (S1 sized task
and trigger coverage at 61–67%, and the rest of the programme is sized
against that figure); verdicts per week are
counted by channel; readings vary run to run; a closure from any surface
appears in `sessions appraise`; the recorded brief is complete on a sample of
runs.

#### Phase 1 as pull requests

Each is independently reviewable and lands behind its own tests; none
changes what a run does. Dependencies are the only ordering.

| PR | scope | proposals | depends on | acceptance |
|---|---|---|---|---|
| **1a** | **Goal anchors from structure.** `GoalRef` gains `trigger` and `request` kinds (lenient on read — a closed enum in an append-only store is a wire format). `tasks work` seeds the anchor to `task:<id>` with the project as parent; a trigger run anchors to `trigger:<name>`; a trigger's optional `serves` is validated against the loaded charter at load; the front-door drain seeds `request:<id>`. | S1, R1 | — | a delegated and a trigger run each record a non-null anchor; `sessions health` shows them; a `serves` naming a missing line refuses the trigger at load |
| **1b** | **Task closure as a recorded event, core half.** One closure function; the append-only closure record (transition, actor, surface, sessions, time, reason); `pre_task_close` (may deny, fails closed), `task_closed`, `task_reopened` hook events; the CLI, TUI and web board call it; the web shows the readout from the record; an unattended or delegated run cannot close. | S8, R15 | — | every direct surface writes one record per transition; a reopen is recorded and joined to its closure; a denying `pre_task_close` blocks; an unattended close is refused on every route |
| **1b-2** | **Posture from the harness, not the environment** (review of #293; owner's option A, 2026-09-24). The shell tool registers every command it spawns — the child's pid, the run's posture, the session — in a harness-written registry under `~/.mecha`, removed when the command exits. `mecha tasks set` takes its posture from the nearest registered ancestor, so a command that sets `MECHA_RUN_POSTURE` itself changes nothing; the variable becomes advisory, and a process that claims a run no marker confirms is refused. Registering the *shell child* rather than the hosting process keeps an owner's web-board closure (a child of `serve`) from being refused while `serve` hosts a delegated chat. `mecha doctor` reports a `[sandbox]` that mounts the mecha home, the setting the guard leans on; an unconfined `shell` is the stock default, shown by `mecha tools` rather than a doctor finding (review of #294). Residue, named: a command that detaches from its shell escapes the ancestry. | S8 | 1b | a delegated shell's `MECHA_RUN_POSTURE=interactive mecha tasks set …` is refused; an interactive shell's close records `owner-approved`; a claimed posture with no marker refuses; a web-board close during a delegated web chat succeeds; dead markers are skipped; the doctor finding fires on a sandbox that mounts the home |
| **1c** | **Closure from Slack and the graph TUI.** Slack's existing `TaskDone` tap re-routed through 1b's event and tagged `surface: slack` (#293 already passes `--surface slack`), plus a new `TaskDrop`; the Slack reply carries the readout from the record, since the executor parses stdout; the graph TUI's status change calls mecha's closure (a mecha-graph PR). | S8, here §5 | 1b | a Slack and a graph-TUI closure each produce a record with the right surface |
| **1d** | **Read the verdicts already given.** Reopen signs per R16 from 1b's record; the reject reason reaches the reflector as an owner correction; workflow close / cancel / reopen / verify sign per R16b–e; rule and reflection curation and harness accept / reject / revert are recorded against the rule, reflection or candidate (R16f–h); graph review rejections of facts from `agent:mecha` episodes are recorded for L7. | S3a, R16 | 1b | `sessions appraise` shows each new channel on a fixture; none of R16f–h moves a run's valence |
| **1e** | **Readings per item.** Charter and backlog readings carry per-item age and count beside the level, and each run's delta; a line saturated for `SATURATED_AFTER_RUNS` is withdrawn from in-run consumers and reported once by the doctor. | S5 | — | a fixture store with one stale and several fresh items: the level reads saturated while the per-item reading and the per-run delta change as items are added and cleared; the saturated line is withdrawn and reported once (on the live store, the same should be visible run to run) |
| **1f** | **One commitment record, guilt per commitment.** `workflow::Commitment` absorbs `anticipation::Commitment`; drafts, parked questions and accepted front-door requests are commitments by construction; guilt per item = excess over patience × line rank; `anticipated_guilt` becomes a readout (the maximum), with old records still readable. | S7, R12 | 1e | each pending commitment has its own value; the homeostat readout matches the maximum; no consumer reads the scalar |
| **1g** | **Store every counterfactual comparison.** Steer-probe and validation verdicts today, and the comparisons phases 2, 3 and 5 add, write one record each: situation keys, goal kind, call class, the arms, the deciding validator, the verdict, pointers — clean sessions with a readable tool surface only. | X1, O4 | — | a `--probe` run leaves records a second read returns; tainted sessions leave none |
| **1h** | **The situation brief, assembled and recorded.** The goal chain, a harness-side board read reduced to counts and pointers, per-commitment readings (after 1f), local time and quiet hours, seats and runs in flight, budget — recorded on the run, delivered nowhere. Small readers for `/slots` occupancy and a voice call in progress. | B1 | 1a (1f for commitments) | on fixture runs of each kind (delegated, trigger, web) with a seeded board, charter and backlog, the recorded brief carries every field; no brief text appears in any provider request |
| **1i** | **Numbers never reach the model as text.** Half exists: `planning_sensor_metadata_never_reaches_either_provider` (`provider/anthropic.rs`) already proves `Message::planning` metadata is byte-identical out of both encoders. The gap is a sensor number, setpoint or numeric valence arriving as *block text* — a status line, a leaked brief. Add that test beside the existing one. | G4 | — | the new test fails when a status line carrying a sensor reading is injected into a tool result or user turn; a second check scans a real recorded request (a fixture run through both encoders) for any sensor number, setpoint or valence, so it can fail on a leak nobody thought to inject; the existing metadata test still passes |

1a, 1b, 1e, 1g and 1i can proceed in parallel; 1h follows 1a. The phase-1 readout — anchored
share of long runs, verdicts per week by channel, per-item reading variance —
is added to `sessions health` by the PR that first produces each number.

### Phase 2 — One interpretation, and learning from it

*Offline. Serves self-learning, and produces the goal hypotheses alignment
needs.*

| # | work | proposal |
|---|---|---|
| 2a | **The distiller, extended, becomes the appraisal.** New inputs: the goal chain and charter text, the situation brief at start and finish, the owner's acts on the output (release, edit diff, reject reason, closure, reopen), signed errors, step findings and probe verdicts, up to three past clean appraisals of the same situation and goal. New outputs, in `appraisals.jsonl` beside the unchanged graph episode: the interpretation, good/bad per goal with pointers, lessons, a prediction for next time, goal hypotheses. The counts-only appraiser is retired into it | I1, I4 |
| 2b | Predictions scored when next time comes; a miss is a surprise that raises the episode's priority | X5 |
| 2c | Past clean appraisals retrieved by situation and goal through `goal_context`, with the goal as a `Situation` key | I2, M1 |
| 2d | **Point-wise counterfactual comparison, beside whole-session rumination** (R26: both): at the informative decision points of recorded sessions (a steer, a denial, a failed check, an edited or rejected draft, a surprise), drive K policies a short horizon from the point and let the owner's recorded verdict decide; the losing arms' confirmed outcomes are written into that session's appraisal | O1, O3 |
| 2e | Learning from appraisals: the reflector's lessons measured against I1's on the same interventions, then `learn` fed clean appraisals — successes included, corrections attributed data / behaviour / gap, tenure by a Wilson bound on the owner's verdicts behind R20's guard, priority from I1's judgments × how often the situation recurs | L2, L7, L3, L1 |
| 2f | The nightly diagnostician reads clean appraisals beside its counters — *built (R38)* | L8 |

**Done when**, in a lifetime experiment on fixtures against the
appraisal-off preset: lessons from appraisals validate more often than
reflector lessons; predictions score above chance; point-wise comparisons
reach decisions where whole-session pairs tied; verified task success does
not fall.

#### Phase 2 as pull requests

Same form as phase 1's: each independently reviewable, behind its own
tests, and dependencies the only ordering. Every row that changes what a
later run or the learner sees ships first in shadow and then as a lever
with a `mecha exp` arm against EXPERIMENT-DESIGN §15's appraisal-off preset
(decision 7); nothing reads an appraisal except through
`appraisal_store::Clean`, save the owner's own surfaces (R19).

| PR | scope | proposals | depends on | acceptance |
|---|---|---|---|---|
| **2a-1** | **The text-appraisal record and store.** `appraisals.jsonl` beside the graph episode: one bounded interpretation, good/bad per goal, the claims it rests on (each a pointer and a quote), a prediction, goal hypotheses, lessons — labels only as words in the prose, no scalar. The write door grounds each claim through `grounding::admit` against what the run received (call results, the owner's turns — never the agent's own words) and drops, before storage, any that does not dereference, counting it by reason on the record; it stamps the taint and `Origin` read off the transcript, failing closed; a tainted run's appraisal is stored. Two read doors: `clean()` returns `Clean`, a type only the store can make, for learning, retrieval, credit and tenure; `for_owner()` returns every record. The graph episode's text pinned. `sessions appraise` counts the store. No producer. | I1, R17, R18, R19 | — | an appraisal of a clean fixture session reads back through a fresh handle via the clean door; one of a tainted session is stored and never returned by it; a claim whose pointer does not dereference is dropped before storage and counted; old and unknown variants load leniently, an unreadable origin as untrusted |
| **2a-2** | **The distiller, extended, writes the appraisal** — in shadow: the store gains its producer and nothing reads it but the owner. New inputs to the pass: the goal chain and the charter's text (the anchor, 1h's recorded goal chain); the recorded brief at start and the homeostat at finish (1h); the owner's acts on the output — release, edit diff, reject reason (R16a), closure and reopen (1b, 1d); the signed errors (`appraisal::of_session`); step findings and the session's stored comparisons (1g); up to three past clean appraisals of the same situation and goal, through `Clean` only. The transcript it reads carries the referent ids the store dereferences (`result:<id>`, `turn:<n>`); a quote is a span of the whole result, not of the 300-character clip the renderer shows today. Judgments' goal references are resolved against `distill::KnownPointers` before recording. Runs where `distill` already runs, on the local model (R29), under a permit. The owner's readout of the prose (every appraisal, control characters stripped). | I1, I4, R18, R25, R29 | 2a-1 | on clean and tainted fixture sessions with a fixture model, each appraisal lands behind the right door; a past clean appraisal of the same situation reaches the next session's input and a tainted one never does; both R25 pins pass (R32: the appraisal is a follow-up turn on the cached prefix); seconds of a seat per session measured on real sessions — *built; see I1* |
| **2a-3** | **The counts-only appraiser retired into it.** `sessions appraise --appraise` and `appraise_with_model` go; the counts it read (`AppraiserEvidence`) are already 2a-2's input as signed errors. Records carrying `channel: appraisal` / `cite: appraiser` still load and count. | I1, R25 | 2a-2 | no second model pass reads a session for the label; an old record with an appraiser error loads and is counted — *built; see I1* |
| **2a-4** | **The reflector folded in** — only after 2e-1 measures its lessons no worse (R25). A reflection is an appraisal of a correction: the same pass writes both, still as a `Reflexion` with its `Origin`, so `learn`'s input and gate keep their shape. | I1, R25 | 2a-2, 2e-1 | 2e-1's measurement is on record; model passes per session fall from two to one; the learning store's provenance gate is unchanged (its tests pass untouched) |
| **2b-1** | **Anticipation's predictions scored.** Every `Prediction` an `Outcome` resolves is a calibration point per kind; coverage is reported, never a calibration figure while outcomes are absent; a delivery positive only after `outbox reconcile`. | X5 | 1d | a fixture store with resolved and unresolved predictions reports coverage per kind and no rate over nothing — *built; see X5* |
| **2b-2** | **The appraisal's own prediction scored** once the owner's act on the appraised session's output arrives, or R37's window closes; a miss is a surprise, recorded for 2e-6's priority. The structural scorer is R33 (the owner's ruling of 2026-09-25): the record's `expected_act` (R16's closed set, added by 2a-2) against the owner's recorded act on the appraised session's output, with "no act" resolved by R37's window; the prose prediction is never scored. | X5 | 2a-2, 2b-1 | a fixture pair of sessions scores a hit and a miss on `expected_act` against the recorded act; a model never decides a score (R27); "no act" resolves only after R37's window — *built; see X5* |
| **2c-1** | **The goal joins `Situation`** as a recorded and scope key — recording, matching, replay and validation in one change; an absent goal never widens a scope. *Built as 2c-1 (2026-09-25): the key is the whole `GoalRef` the front-end handed `prepare`, recorded as `RunConfig::rules_goal`.* | M1 | 1a | the scope-key tests cover the goal on every door; a rule mined with no goal still matches as before |
| **2c-2** | **Past clean appraisals retrieved.** `goal_context` serves up to three clean appraisals of the same situation and goal, on demand, never pushed — through `Clean` only. Measured against a control at matched budget, since retrieved memory can cost more than it returns. *Built as 2c-2 (2026-09-25): `Lever::PastAppraisals`, shipping off; the measured run is owed.* | I2 | 2a-2, 2c-1 | a clean appraisal of a matching session is served and a tainted one never is; the lever's arm runs against the control |
| **2d-1** | **Point-wise comparison at informative decision points.** At a steer, a denial, a failed check, an edited or rejected draft, a surprise: `probe::drive_arm` runs K policies a short horizon from the point, and the owner's recorded verdict decides (new: a branch's draft against the released text). Each writes a 1g `Comparison` of a new kind. Points drawn uniformly until 2e-6 ranks them — *ranked since 2e-6 for `sessions compare` only; a harness candidate's points stay uniform (R39)*. **Built as 2d-1** — `mecha sessions compare`; see O1 for what was built and what it left. | O1, R26, R27 | 1g | fixture points of each kind leave comparisons a second read returns; a point whose verdict no structural validator can pose is inconclusive, never judged |
| **2d-2** | **The acceptance combination** (R26): a harness candidate is accepted when the point-wise comparison decides for it and the whole-session numeric comparison shows no regression, `WORK_FLOOR` intact. **Built as 2d-2**, with R36's completion: point-wise against rejects, and point-wise undecided leaves the numeric verdict unchanged. | O1, R26 | 2d-1 | a candidate that wins point-wise and regresses the floor is rejected; one that wins point-wise and holds is accepted |
| **2d-3** | **The losing arm teaches.** A comparison's confirmed losing outcome is written into that session's appraisal as counterfactual reflection — a new pointer kind naming the comparison, which 2a-1's `Pointer::Unread` already round-trips. | O3 | 2a-2, 2d-1 | a decided comparison's loser appears on the session's appraisal, pointing at its comparison; an undecided one writes nothing — *built; see O3* |
| **2e-1** | **The reflector's lessons against the appraisal's**, on the same interventions, by the validation probes already built — shadow, measurement only. R25's gate for 2a-4. **Built as 2e-1** — `mecha learn --compare-sources`; see L2 for what was built and what it left. | L2, R25 | 2a-2 | a report per intervention region: validation rate of each source's lessons, with the counts beneath it — *built; the measurement on real sessions is owed, and it is R25's gate: 2a-4 waits on the appraisal's rate being no worse than the reflector's over the same decided interventions* |
| **2e-2** | **`learn` fed clean appraisals** — lessons and interpretations as material, successes included, through `Clean` only; a stage lever with `stages_off` against reflector-only learning. | L2, R19 | 2e-1 | a tainted appraisal's lesson never reaches a batch (a test on the type); the lever's arm runs |
| **2e-3** | **Attribute a correction by what the run was given** — mecha-graph's D3 contract ported: data error, behaviour error or gap from `grounding::calls`; a behaviour rule mined only from a behaviour error; a gap a retrieval target. | L7, here §5 | 1d | fixture corrections of each class are routed to their class; no behaviour rule is mined from a data error or a gap — *built; see L7* |
| **2e-4** | **Learn from what went right**: owner-verified positives (sent unchanged, answered, `done` and not reopened) as writing exemplars, planning success examples and contrast evidence; a staged skill draft after k successes in one region, proposed only. | L2 | 1d | a draft sent unchanged is mined as an exemplar; a success the owner later reopens is withdrawn; no skill is written without the owner |
| **2e-5** | **Goal-stamped reflections and per-line tenure**: the anchor as the second source of `Reflexion::goals`; tenure by the Wilson lower bound of the owner-accept rate on the line's owner-verdict channels (`ladder.rs` ported); dormancy for a region that stops recurring (`decay.rs`). Appraisal-weighted tenure only behind R20's guard — the owner's verdict overrides, grounded claims from clean runs only — and as a measured lever against owner-only tenure, with a revert, before it is on. | L3, R20, here §5 | 1a, 1d; the appraisal-weighted half 2a-2 | a rule's tenure moves on owner verdicts by the bound, not a streak; an owner verdict overrides an appraisal's bad; the lever reverts |
| **2e-6** | **Replay priority is gain × need**: \|signed error\| on owner-verdict channels × charter rank × how often the `Situation` region recurs (the Selector's demand term) × age decay; the hopeless demoted; the holdout still drawn uniformly first; the same order for `learn`'s batches and the validation budget; 2b-2's misses raise it. | L1, here §5 | 1g; 2b-2 for the surprise term | the uniform holdout is unchanged by the ranking; a recurring region outranks a one-off of equal error — *built under R39; see L1: the Selector's `ln(1 + touches)` ported as the need term, over runs matched in the region* |
| **2f** | **The diagnostician reads clean appraisals** of the episodes its draw selected, beside its counters, through `Clean` only; `carries_over` covers their text as a source; `candidate::judge` still decides. *Built as 2f, under R38: the draw's first phase (pool and uniform holdout) precedes the diagnosis, and the appraisals are the remainder's, never the holdout's.* | L8, R38 | 2a-2 | a tainted appraisal never reaches `diagnose::Evidence`; a proposal lifting a run of words from an appraisal is refused — *built; see L8* |

**Parallel now**, on phase 1 alone: 2a-1, 2b-1, 2c-1, 2d-1, 2e-3, 2e-4,
the owner-verdict half of 2e-5, and 2e-6 without its surprise term.
**After 2a-2**: 2a-3, 2b-2, 2c-2, 2d-3, 2e-1 and 2f, in parallel. **After
2e-1**: 2a-4 and 2e-2. 2d-2 follows 2d-1. The phase-2 readout — appraisals
by door, claims dropped by grounding, lessons validated by source — goes in
`sessions appraise` from the PR that first produces each number, as
phase 1's did.

**Two questions the plan left to the owner, both ruled 2026-09-25 (here §6,
R32 and R33):**

1. **What R25 pins, and whether decision 4's "no extra model call" still
   holds.** The graph extracts facts from the episode, which is the model's
   answer to `DISTILLER_SYSTEM` over the rendered transcript. The two
   options were to extend that one reply, or to keep the prompt
   byte-identical and ask for the appraisal in a follow-up turn on the same
   cached prefix. *Ruled: the follow-up.* The episode's prompt and the
   graph episode stay byte-identical, and decision 4 is amended to one
   extra model call per session. `the_distillers_episode_prompt_is_pinned`
   stays as written and passing. 2a-2 proves the prefix is reused on the
   bytes the local encoder sends, and measured the cost (the 2a-2 row).
2. **How a text prediction is scored.** X5 as written scores anticipation's
   typed predictions. A free-text prediction has no structural validator,
   and R27 lets a model judge at most break a tie. *Ruled: a closed-set
   expectation beside the prose.* This is `TextAppraisal::expected_act`,
   one of R16's owner acts: released unchanged, edited, rejected, closed,
   reopened or no act. 2a-2 added it, lenient on load (`unknown` for a
   word this build cannot read), with no migration. 2b-2 scores it against
   the owner's recorded act.

### Phase 3 — Meaning in the run

*Serves planning, reasoning and completion.*

| # | work | proposal |
|---|---|---|
| 3a | The situation brief delivered at run start, folded into the seed or first user turn — including a re-delegated task's previous attempts and why they were rejected. *Built as 3a behind `situation_brief` (ships off); the previous attempts deferred to 3a-2* | B1, I3, M5 |
| 3b | The agent's situation appraisal, in the run's own slot, after a surprise and before a consequential act | I3 |
| 3c | Planning as joint optimization in the agent's reasoning, over every live goal and the described state; charter rank resolves conflicts | P1 |
| 3d | **Plan-time comparison**: on anchored delegated and trigger runs, two candidate plans as text, validated deterministically — tracing to the goal, coverage of declared criteria, budget fit, charter conflicts, what won at similar points before; the loser kept as the fallback | N1 |
| 3e | Honest completion: checks the harness writes, criteria the agent declares (one-sided), the goal validator, the certificate appended by the harness, drafts from uncertified runs held, the review showing appraisal and plan beside the draft | C2, L4, S6, V1, C1, G2, U2 |

**Done when:** at matched budget, runs with 3a–3b beat the same runs
without; runs with 3d beat single-plan runs; false completion falls with
`WORK_FLOOR` holding.

### Phase 4 — Alignment and the scheduler

| # | work | proposal |
|---|---|---|
| 4a | The owner's goals, as hypotheses from phase 2, retrieved into planning as "the owner seems to want", confirmed or contradicted by later acts, and shown beside the charter | I4 |
| 4b | Confirmation of an interpretation or a plan for the small set of actions that warrant it, riding on the existing review | I5 |
| 4c | Commitments from owner acts: mail `reply` / `task` / `schedule`; promises in released drafts recorded automatically | S4 |
| 4d | **One scheduler**: permits, what to surface and when, duty runs that prepare follow-through, the nightly budget — one objective recomputed as state moves, replacing four rules | P2, A1, U1, U4 |

**Done when:** owner-side latency on commitments falls; interruptions per day
do not rise; goal hypotheses the owner's acts confirm outnumber those they
contradict.

### Phase 5 — Guardrails and mid-run policy change

| # | work | proposal |
|---|---|---|
| 5a | Wind down before the ceiling; park a delegated run as a question instead of dying | C4, A3 |
| 5b | The frustration ladder and the desperation brake | C5 |
| 5c | A send whose recipient does not trace to the goal is staged; destructive calls under taint or an unconfirmed goal are prompted | G1, G3 |
| 5d | Pre-action markers from the stored comparisons, narrowing only | X2 |
| 5e | **Mid-run policy change**: on a harness-computed trigger (a failed check, frustration, a surprise, a budget shortfall), branch two dry continuations for a bounded horizon — reads live, writes to one per-branch upper layer shared by `shell` (a bubblewrap ≥ 0.10 overlay) and the file tools (a copy-on-write layer at `ToolCtx::resolve`), R28, sends to a scratch outbox, un-stageable egress ending the branch — validate structurally, commit the winner, and write the loser into the appraisal | N2 |

**Done when:** on the AgentDojo suite, attack success falls and utility
holds; per arm, tampering and reopen rates fall; N2 beats no-branching on the
fixtures where a structural validator exists.

---

## 4. Parked, and what would unpark each

| item | why parked | unpark when |
|---|---|---|
| fixed advice text (`goal_guidance`, C6, M3 gap delivery) | measured to hurt as often as help | phase 3's situation appraisal is measured and a place for advice is argued |
| S2 — inferring a goal for un-anchored interactive runs | goes beyond §17.3; a model pass | phase 1 shows how many long web runs stay un-anchored and phase 2's goal hypotheses prove useful |
| C3 seeded plans; V2 re-ask and drift event | plans can hurt small models; no drift rate yet | phase 3's criteria and plan comparison produce plans to read |
| M2–M4 memory (earned salience, gap delivery, criteria across compaction) | covered for now by I2's retrieval | I2 is measured |
| X0 as a separate pass; X3 verdict forecasts; X4 recorded pre-mortem | X0 is subsumed by O3's counterfactual reflection; X3–X4 need stored comparisons | phase 2 holds comparisons and scored predictions |
| N3 — world-model lookahead (the model imagines outcomes); speculative execution | no structural validator for imagined futures; speculation is a latency tool | a structural validator exists for the decision, or latency becomes the problem |
| A2 earned autonomy; A4 curiosity; L5 surprise-seeded gossip; L6 lineage | lower value, or a new use of slack | phases 2 and 4 are measured |

---

## 5. mecha-graph: port on demand

The owner's direction (2026-09-24): mecha is the harness and mecha-graph a
tool that should eventually merge into it. A graph mechanism moves into
mecha core **when a phase needs it**, using the graph's version as the
reference; no new cross-repo reader is built as the long-term shape.
Inventory §5 has the overlap table.

| phase | what it ports |
|---|---|
| 1 | the board's closure path, onto the one closure event (S8) |
| 2 | the D3 correction contract (*ported as 2e-3*); the ladder's Wilson-bound tenure; decay as rule dormancy; the Selector's demand term as L1's *need* |
| 3 | `verify.rs` folded into `grounding.rs` — one grounding primitive |
| 4 | review-on-use's verdict queue as the shape of `mecha review` |
| 5 | pack flags (contradicted / denied / stale) as evidence for the brief and the markers |

---

## 6. Rulings

Numbered as before, so earlier answers still cite them. None is a security
widening.

| # | phase | ruling | status |
|---|---|---|---|
| R14 | all | Mechanisms overlapping mecha-graph are built in mecha core, porting the graph's version | **stated by the owner** |
| R17 | all | An appraisal is an interpretation of meaning, in text; good/bad, goal and pointers are judgments read out of it; labels incidental | **ruled** |
| R24 | 3–4 | The optimization is joint over every need; charter rank resolves conflicts, so goals cannot stalemate | **ruled** |
| R1 | 1 | A trigger run anchors to the trigger itself; a charter `serves` link is optional | **ruled** |
| R12 | 1 | Guilt per commitment, one commitment record | **ruled** |
| R15 | 1 | Task closure and reopening, on any surface, are one recorded event with hooks | **ruled** |
| R16 | 1 | How the unread acts sign: reopen −1.0 at any age; graph fact rejections to attribution only; R16a–h as tabled in S3 | **ruled** |
| R18 | 2 | The appraiser reads the full transcript; the appraisal inherits the run's taint | **ruled** |
| R19 | 2 | A text appraisal may reach learning, memory retrieval, the owner's surfaces, and credit and tenure — clean runs only for all but the surfaces | **ruled** |
| R2 | — | A one-tap verdict | **declined** |
| R7 | — | Draft expiry | **deferred** until the system stabilises |
| R20 | 2 | The guard on credit and tenure from text: the owner's verdict overrides; grounded claims from clean runs only; a measured lever with a revert first | **ruled 2026-09-24** |
| R25 | 2 | I1 is the distiller extended; the counts-only appraiser is retired into it; the reflector folds in only after its lessons measure no worse; the graph episode's text stays unchanged | **ruled 2026-09-24** |
| R26 | 2 | Point-wise comparison at informative decision points, decided by the owner's recorded verdicts, is added **beside** the existing whole-session numeric comparison, which stays; behaviour-changing policies are also measured on fixtures. They combine as O1 sets out: a candidate is accepted when the point-wise comparison decides for it and the numeric comparison shows no regression (`WORK_FLOOR` intact) | **ruled 2026-09-24: both, combined as O1 proposes**; refined by **R36** (what happens when the point-wise comparison does not decide) |
| R36 | 2 | R26's combination, completed: point-wise **for** the candidate (at least 4 decided points, strictly more candidate-only passes than baseline-only ones, no separate holdout) and **no numeric regression** — work above `WORK_FLOOR`, no unpredicted metric past `REGRESSION_CEILING`, the predicted metric not worse in either slice; "did not beat the original" is a missing win, not a regression — **accepts**, when the class may be accepted by measurement at all; point-wise **against** **rejects** whatever the numbers say; point-wise **undecided** leaves the numeric gate's verdict **unchanged**, recorded as numeric only, which keeps the 2026-08-22 auto-accept of config rumination. Up to 8 points per measured candidate, inside `harness measure`, under 2d-1's seat rules | **ruled 2026-09-25** (the owner, on 2d-2's shape question); built as 2d-2 |
| R21 | 3 | State reaches the agent as described state, on the user-turn or tool-result slot, never the prefix. Budget *facts* may be numbers — turns left, context remaining; anything a model could treat as a *score to move* stays words: sensor readings against setpoints, per-commitment guilt, valence, priorities | **ruled 2026-09-24** |
| R22 | 3 | An in-run situation appraisal is part of the run: it inherits its taint, shapes the plan, and never widens a permission or chooses an action | **ruled 2026-09-24** |
| R35 | 3 | Delivering the situation brief arms `private` taint — fail-closed, the same as reading the board through `kg_task_list` (raised on review of #309) | **ruled 2026-09-25**; built as 3a-3 |
| R27 | 3, 5 | Policies are compared only where a structural validator decides; a model judge at most breaks a tie between candidates that passed every structural check | **ruled 2026-09-24** |
| R4 | 3 | The completion certificate: template only first; a `Verify` re-prompt only later, as its own measured arm | **ruled 2026-09-24** |
| R11 | 3 | Acceptance criteria the agent declares, from a closed set the harness executes; frozen; one-sided until the owner confirms them; never a charter sensor | **ruled 2026-09-24** |
| R23 | 4 | Confirmation of an interpretation or plan only for irreversible or outward acts (on the existing review), a delegated task whose interpretation departs from its anchor, and charter conflicts rank cannot settle | **ruled 2026-09-24** |
| R10 | 4 | Promises in released drafts recorded as commitments automatically; dismissing one drops it | **ruled 2026-09-24** |
| R5 | 5 | Desperation brake: refuse writes to a frozen check's read set; withhold `Complete` after two failures | **ruled 2026-09-24** |
| R6 | 5 | A recipient that does not trace to a confirmed goal is staged | **ruled 2026-09-24** |
| R13 | 5 | Stored comparisons may narrow a matching call before dispatch | **ruled 2026-09-24** |
| R28 | 5 | Mid-run counterfactual branching is built. Each dry branch gets **one upper layer over the workspace, shared by `shell` and the file tools**: bubblewrap mounts it as an overlay for `shell` (`--overlay`; needs bubblewrap ≥ 0.10 — this box's 0.9.0 has no overlay options, checked 2026-09-24; 0.13.0 installed since 2026-09-25, see below the table — built from upstream and installed alongside), and the file tools, which write in mecha's own process outside any bubblewrap namespace, resolve through the same directory at `ToolCtx::resolve`. Preflight refuses to branch — never falls back silently — where either half is missing. The bubblewrap upgrade covers only the `shell` half; the copy-on-write layer in the file tools is harness code (found on review of #291) | **ruled 2026-09-24**, refined on review: both halves required |
| R3 | parked | Inferring an anchor for un-anchored runs onto a closed list of pointers | parked |
| R8 | parked | The harness may *propose* per-region autonomy grants | parked |
| R29 | — | Sending transcripts to a cloud model for interpretation or rollouts | not proposed; the owner's privacy decision |
| R9 | — | The charter line `be-the-best`: unboundedness is fine (lines are attractors); §15's narrower worry is a line whose object is the harness, held by the `Security` class | flagged once |
| R30 | 1 | The graph TUI closes and reopens through mecha's closure event only when the owner's install opts in (`[board] close_through` in `~/.mecha-graph/config.toml`); opted in, it fails closed, refusing with nothing written when mecha is missing or the TUI is not on the default database; not opted in, standalone mecha-graph keeps its direct write. Option A3 of #300's five, not the catalogue's A3 | **ruled 2026-09-25**; built as mecha-graph#21 |
| R31 | 1 | Commitments: new writes only, no migration — new predictions write `workflow::Commitment`, old shapes stay on disk and read leniently. Its `due_at` and `follow_up_at` are optional, an absent one meaning "no deadline stated"; the harness never derives a date, and dates the owner wrote on evidence pass through (S7) | **ruled 2026-09-25**, the dates refined on review of #304; built as 1f-2 (#304) |
| R32 | 2 | What R25 pins: 2a-2's appraisal comes from a follow-up turn on the same cached prefix, so `DISTILLER_SYSTEM` and the episode stay byte-identical; the extra model call is taken deliberately over changing the episode's text (here §3, the first of phase 2's two questions) | **ruled 2026-09-25** by the owner directly; built as 2a-2 (#314), which amends decision 4 to one extra model call per session |
| R33 | 2 | How a text prediction is scored (2b-2): a closed-set expected owner act from R16's set sits beside the prose and is scored structurally against the act the owner records (here §3, the second question) | **ruled 2026-09-25** by the owner directly; the field is `TextAppraisal::expected_act`, added by 2a-2 (#314); scoring it is 2b-2 |
| R34 | 2 | A rule scoped to a goal that closes keeps its scope (`task:<uid>`) and widens only on evidence, by §17.4's restatement; such rules are made **visible**, not left silent | **ruled 2026-09-25; built as #317** — `mecha rules list` counts and marks them `LOADS NOWHERE`, `mecha learn` repeats the count each pass, an unreadable board is its own finding |
| R37 | 2 | An appraisal's expected owner act of "no act" becomes the act that happened once **the output's store patience** has elapsed, counted from **the appraised session's end**: the patience is `doctor::Patience::for_store`'s (the charter line watching that store, else the doctor's constant); an output with no store (a chat answer) resolves at the doctor's constant; an owner act that arrives before the window closes is the act; an unreadable act store or patience is unknown and never resolves to no act. **Refined 2026-09-25:** a task's output uses the task's due date — the window runs from the session's end to the board row's `due_at` (the end of that day in the owner's zone), an owner closure by then is the act, an undated task keeps the constant, a `due_at` already past at the session's end falls back to the constant, and an unreadable board or unparseable `due_at` is unknown; workflow outputs keep the constant for now; "the doctor's constant" is confirmed as the outbox's 48h | **ruled 2026-09-25**; built as 2b-2 |
| R38 | 2 | Which episodes the diagnostician's appraisals come from (2f): the nightly's draw is split in two, on one seed. The candidate id — the seed — is minted before the diagnosis, and the eligible pool and its uniform holdout are drawn then, since neither reads the metric; the diagnostician reads the clean appraisals of **the pool minus the holdout**, and **never the holdout**; after the proposal the selection is ranked by headroom from that same remainder, exactly as before, so `judge_drawn` and `combine` get the inputs they got. Accepted costs: a pool walk every night (no model call) and a candidate id minted and discarded on nights with no candidate. `mecha diagnose` run by hand has no draw and carries no appraisals. A brief carrying one opens the diagnostician's conversation **private**, fail-closed, and the thinner research on those nights (after the first fetch, blind `web_search` only) is accepted | **ruled 2026-09-25/26** (the owner, on 2f's shape question); built as 2f |
| R39 | 2 | How 2e-6's replay priority enters the two draws that feed a gate. **The harness selection**: headroom on the predicted metric above zero is the gate — an episode that can only tie comes after every one that can discriminate — and within each part the order is the priority, then the charter rank, then the id. **Point-wise points**: `mecha sessions compare` ranks its uniform draw by the priority of each point's session; `compare_candidate` inside `harness measure` stays uniform, because its points are R36's confirming sample (no separate holdout) and ranking them would bias the verdict (GOAL-SYSTEM-DESIGN §8.1) | **ruled 2026-09-26** (the owner, on 2e-6's two shape questions); built as 2e-6 |

**Every ruling is settled** (2026-09-24; R30–R37 on 2026-09-25; R38 on 2026-09-25/26; R39 on 2026-09-26), except the
parked items (R3, R8), the flag (R9), the deferred R7, the declined R2 and
R29, which is not proposed.
Phase 5's R28 waited on the bubblewrap upgrade, an ops step; the workstation
has run bubblewrap 0.13.0 from `/usr/local/bin` since 2026-09-25
(`bwrap --version`, and #295 measured under it), so the prerequisite is met;
both halves, the `--overlay` mount and the file tools' copy-on-write layer,
are unbuilt.

---

## 7. How it is measured

Outcomes the whole programme is judged on, from APPRAISAL-RESEARCH §8.5 and
EXPERIMENT-DESIGN Part II: independently verified task success, per-charter-line
owner verdicts (sent-unchanged and rejection rates), false completion, asks per
run, rework, interlock friction and dojo attack success, latency and tokens —
all at matched budget, with variance. **Not** the label distribution, raw
valence, lower sensor values or rule count; those are the instrument, not the
outcome.

**The instrument already exists; use it.**
- **Run levers and stage levers.** In-run proposals are `harness::Lever`s;
  the nightly ones (L1, L2, L3, L7, X1, X5, S5's saturation handling) are
  stage levers measured with `stages_off`, as `sensors_in_brief` already is.
  EXPERIMENT-DESIGN §15's appraisal-off preset is the control arm.
- **`WORK_FLOOR` guards the narrowing proposals.** C5, G1–G3 and V2 can win a
  comparison by doing less; the floor (0.75) and "an unfinished correction is
  a regression, never an efficiency win" are what stop that.
- **The readouts:** `mecha learning-report` gains owner-verdict and valence
  lines and becomes the programme's outcome readout;
  `scripts/appraisal-{validity,report,traces,learning-report}.py` and the
  `eval/appraisal-*.toml` manifests are the existing harness for arms; the
  synthetic home's `[fixtures] charter` pins a charter per trial.
- **The confirmation surface is `mecha review`** for S2, S6, R10 and A2.

### What each proposal buys, by the owner's seven axes

| | capability | accuracy | performance | independence | security | self-learning | usability |
|---|---|---|---|---|---|---|---|
| S1 anchors | ● | | | ● | ● | ● | |
| S2 harness asks | ● | ● | | | | ● | ● |
| S3a verdicts already given; S8 closure event | | ● | | | | ● | ● |
| L7 D3 attribution | | ● | | | | ● | |
| here §5, porting mecha-graph | | | ● | | | ● | ● |
| S4 commitments | | | | ● | | | ● |
| S5 sensor hygiene | | ● | | | | | ● |
| C1 honest completion | | ● | | | ● | ● | ● |
| C2 harness checks | ● | ● | | | | ● | |
| V1 goal validator | ● | ● | | | | ● | |
| V2 alignment that acts | ● | ● | | ● | | | ● |
| C3 seeded plans | ● | ● | | | | | |
| C4 wind-down | ● | | ● | ● | | | |
| C5 ladder + brake | ● | ● | ● | ● | ● | | |
| G1 send alignment | | | | | ● | | ● |
| G2 embarrassment hold | | ● | | | ● | | |
| G3 risk-weighted approval | | | | | ● | | |
| L1 gain × need | | | ● | | | ● | |
| L2 successes | ● | ● | | | | ● | |
| L3 goal-stamped tenure | | ● | | | | ● | |
| S6 declared criteria | ● | ● | | ● | | ● | |
| S7 guilt as goal error | | ● | | ● | ● | | ● |
| X1–X2 counterfactual markers | | ● | | | ● | ● | |
| X3–X5 forecasts and scoring | | ● | | | | ● | ● |
| M2/M3 salience, gap delivery | ● | ● | ● | | | | |
| M4/M5 goal survives, prior attempts | ● | ● | | ● | | | |
| A1 duty runs | | | | ● | | | ● |
| A2 earned autonomy | | | | ● | | | ● |
| U1 interrupt gate | | | | | | | ● |
| B1 situation brief | ● | ● | | ● | | | ● |
| I1 one informed interpretation | ● | ● | ● | | | ● | ● |
| I3 appraisal while working | ● | ● | | ● | | | |
| O1–O4 point-wise comparison, stored | | ● | ● | | | ● | |
| N1 plan-time comparison | ● | ● | | ● | | | |
| N2 mid-run policy change | ● | ● | | ● | ● | | |
| P2 one scheduler | | | ● | ● | | | ● |

---

## 8. The proposal catalogue

The detail behind each id, grouped by the phase that builds it. Parked
proposals are at the end.

### Background: counterfactual reasoning today

**What exists is two halves that never meet.**

*Retrospective counterfactuals — built, and live in part:*
- **Rule validation** (`mecha validate`, `counterfactual.rs`): branch the
  transcript at an owner intervention, strip the steer, replay with and
  without the rule; the verdict is structural (did the model now do the
  steered thing; did it repeat the refused call). Ledgered per rule and
  region; drives probation and retirement.
- **Steer probes** (`sessions appraise --probe`, `appraisal_probe.rs`): the
  same branch, asking whether the steer was load-bearing — `regret` if the
  unsteered replay went elsewhere, `disappointment` if it got there anyway.
  **Computed on demand and discarded**: the appraisal is never stored, so a
  verdict that cost a model run lives only in one readout. *(Stored since
  1g, #298: each driven probe writes a `comparison::Comparison`; see
  HISTORY, 2026-09-24/25.)*
- **Harness rumination**: paired replay of a config candidate against the
  current harness, gated by `candidate::judge`.
- **Artifact probes** (`probe::prepare_mismatch`): a reflection's lesson
  re-tested on the whole task against pinned gold.
- **Reflection** turns interventions and mismatches into `Reflexion`s, and
  `learn` consolidates them into region-scoped rules.

*Anticipatory appraisal — built, opt-in:* `anticipation::assess` is a pure
function from `Evidence` (commitment, verification state, whether a check
exists and fits the time, budget shortfall) to a set of concern kinds and one
`Response` (`Proceed`, `Verify`, `Clarify`, `Replan`). It runs at outbox
staging (a `Prediction` on the draft, which in `guide` mode blocks release
until the response is `Proceed`) and on plan writes when the owner supplied
evidence. Owner-recorded `Outcome`s resolve predictions later. Its one
counterfactual is `Kind::Regret`: *a named, affordable check exists and you
have not run it* — a comparison of exactly two actions, proceed and check,
with no model of what either leads to.

**The gap.** Every retrospective probe produces the datum anticipation
lacks — in *this* situation, the agent did *A*, the owner wanted *B*, and a
replay showed whether *A* would have led to *B* anyway — and nothing carries
it forward. The design named the bridge and did not build it: §7.4's "fast
pre-action marker: one cheap lookup with two keys — the homeostat for
predicted state, the appraisal store for recorded situations", and §17.4's
"a rule scoped to a tool and a condition renders as one line on that tool's
result the first time the condition recurs". This is the somatic-marker
shape: a fast, learned, situation-keyed signal attached to an action before
it is taken, derived from what that action led to before.

### For phase 1 — foundation

#### S1. Structural goal anchors

**Problem.** Every goal-keyed consumer reads an empty anchor. The harness
already holds the pointer on the runs that matter and throws it away:
`commands::tasks::work_prompt` writes `Id: task-…` into prose and never sets
`Conversation::goal_anchor`; triggers have no goal field at all.

**Build.**
- `tasks work` sets the anchor to `GoalRef::Task(id)` before the run, with the
  task's project as the parent — the same seeding `questions::seed_anchor`
  does on a resume. A board row is owner-created (`kg_task_create`), so this
  is an owner-authored goal, not a model's.
- A trigger run is anchored to **the trigger itself** — a new `GoalRef`
  kind, `trigger:<name>`, lenient on read like the others. No owner work:
  the trigger is owner-written, recurs, and so gives the goal its own
  history the way a board task does. A trigger *may* also carry an optional
  `serves = "charter:<id>"` where the owner wants that link; it is never
  required, is validated against the loaded charter at load, and inherits
  the charter's author rule because the trigger store is owner-only with no
  configurable path.
- The front-door drain seeds `request:<id>` for the privileged run it starts;
  a mail-draft run seeded from a triage verdict carries the thread pointer.
- Web surfaces that start work from an object (a board task, a triage row, an
  outbox item) pass the object's pointer as the anchor. The click is the
  owner's.

**Coverage.** Task and trigger alone cover 48–53 of the 79 long real runs
(61–67%), about a fifth of all runs; the rest of the long runs are
interactive web sessions (S2). Short runs need no goal and get none.

**Class.** Non-adversarial, no model call, one line per front-end. Ruling R1
covers the trigger field.

#### S8. Closing a task is a recorded lifecycle event, with hooks

**Ruled by the owner, 2026-09-24:** "closing a task via a chat, tui, slack,
or web should be recorded and have a hook just like pre/post turn and session
start/end."

**Today.** The closure appraisal lives inside the CLI verb `tasks set`. The
TUI's `/tasks` and the web board reach it (the web through that verb, with the
readout lost on the child's stderr); Slack's Done tap (`Action::TaskDone`)
runs the same verb and loses the readout harder still — its executor parses
the child's stdout and reads stderr only on failure; in chat the
model is refused a direct status write (`ClosedStatusGuard`) and may instead
run `mecha tasks set` through `shell` behind the approver; the graph TUI's
status cycling calls `gtd::set_task_status` and bypasses all of it. Nothing is
recorded: the appraisal is printed and gone, so a closure cannot be read back,
joined to a later reopen, or observed by the owner's own tooling. The hook
events today are `pre_tool`, `post_tool` and `session_end` — there is no
turn-level or session-start event, and no task event.

**Build.**
- **One closure function in mecha core**, which every surface calls:
  transition the board row, write a **closure record**, run the closure
  appraisal, stage at most one follow-up, run the project-closure reading, fire
  the hooks. The record is append-only (`~/.mecha/closures.jsonl`, the
  question store's conventions): task id, the transition (`next → done`,
  `done → next` for a reopen), the **actor** (`owner` on a direct surface,
  `owner-approved` when a model ran it in chat behind the approver), the
  **surface** (`cli`, `tui`, `web`, `slack`, `chat`, `graph-tui`), the sessions
  that worked the task, the time, and the reason if one was given.
- **Every surface uses it.** The CLI and TUI directly; the web board and its
  readout (now read back from the record, so the page can show it); Slack
  gains `TaskDone` / `TaskDrop` in its closed `Action` enum
  (`SLACK-ACTIONS-DESIGN.md`'s shape); chat keeps the approver path and is
  recorded as `owner-approved`; the graph TUI closes through mecha (here §5).
- **Reopening is the same event, reversed**, which is what R16's reopen
  signal reads.
- **A run with nobody present cannot close a task.** An unattended or
  delegated run is refused outright, whatever route it tries — which closes
  the residue `closure_guard.rs` names, where an unattended run holding a
  shell could follow the refusal text to `mecha tasks set`.
- **Hooks**, on the existing hook conventions: `pre_task_close` may deny, and
  fails closed like `pre_tool`; `task_closed` and `task_reopened` observe,
  receiving the closure record as JSON. The owner's tooling can then react to
  a closure — log it, notify, archive — without mecha knowing what it does.

**What the appraisal gains.** A closure verdict that is stored rather than
printed; a reopen joined to the closure it undoes; the surface and actor on
every verdict, so a chat closure the owner approved and one they made
directly can be told apart; and no surface where a verdict is lost.

**Not in this item:** turn-level and session-start hook events. The owner's
ruling names them as the model to follow; they do not exist yet, and adding
them is a separate change to `hooks.rs`.

#### S3. Owner verdicts: collect the ones already given, then add one

**S3a first.** Before any new control, sign the owner acts inventory §4 lists
as unread: a task reopened after `done` (−1.0 on the session that closed it,
and it withdraws that session's success for L2), workflow close / cancel /
reopen / verify, the words of an outbox rejection (to the reflector as an
owner correction), rule and reflection curation (to L3), harness accept /
reject / revert (to L6), and graph review verdicts on facts a session
claimed (joined back by the episode's session id). Each is owner-authored,
already recorded somewhere, and costs the owner nothing new. How each signs
is ruling R16. Ruled: a reopened task (any age) −1.0 on the closing session,
withdrawing its success; a rejected graph fact goes to L7's attribution only.
The rest, ruled 2026-09-24 as proposed:

| # | owner act | proposed signal |
|---|---|---|
| R16a | reject a draft **with a reason** | the reason goes to the reflector as an owner correction (the reject already signs −1.0) |
| R16b | workflow `close` | +0.5, the owner accepted the work |
| R16c | workflow `cancel` | −0.5, owner agency — abandoned, like an abandoned question |
| R16d | workflow `reopen` | −1.0 on the closing session, any age — the task-reopen ruling |
| R16e | workflow `verify` fails / passes | fails: −1.0, mecha's agency; passes: evidence for the certificate, no sign |
| R16f | retire / restore a learned rule | tenure only: retire counts against the rule, restore for it; never a run's score |
| R16g | drop / edit a reflection | a verdict on the reflector: a dropped reflection never becomes a rule, an edited one carries the owner's text; never a run's score |
| R16h | harness change `accept` / `reject` / `revert` | credit for that change and the diagnosis behind it (L6); never a run's score |

**S3b — declined 2026-09-24 (here §1, decision 2).** A one-tap verdict
asks the owner for work the system is meant to infer. Kept below for the
record, with why it was proposed.

**Problem.** The positive channels are starved. Web sessions are 70% of runs
and produce an owner verdict only when they stage a draft; 5 of 120 appraised
sessions reached `pride`. Every reuse-from-experience method with a measured
gain stores successes as well as failures (Agent Workflow Memory +24.6% /
+51.1% relative, arXiv 2409.07429; Contextual Experience Replay +51% relative,
arXiv 2506.06698; ReasoningBank). Those papers grade success with a model
judge; mecha has something better and is not asking for it.

**Build.** A thumbs-up / thumbs-down on the run-end readout (web chip, TUI
badge; voice as "that was right" / "that's wrong" after a turn), recorded as a
new owner-verdict channel on the appraisal record, cited by message index.
Optional, never prompted for, and silence is not a verdict (the rule pending
drafts already follow).

**Why it matters beyond the label.** It is owner-authored, so it is immune to
reward hacking by construction and admissible for rule tenure under §17.2. It
is the success label for L2, the gain term for L1, and the calibration set for
S2 and C5. Ruling R2.

#### S5. Sensor hygiene

**Problem.** Principle 4. Both behaviour-facing sensors are constants on this
install.

**Build.**
- Readings become **per item**: the oldest item's age against the setpoint,
  and the count over it, beside the level. `Decision` and every new consumer
  read the per-item form and the run's *delta* (did this run add to or clear
  the queue), never the level.
- A reading saturated for `reading::SATURATED_AFTER_RUNS` rows is withdrawn
  from in-run consumers and handed to the owner as one doctor finding naming
  the items — the saturation guard already exists and today only reports.
- *Deferred (R7):* an owner-set expiry for drafts, past which a pending
  draft moves to `expired`, signing nothing. Not until the system has
  stabilised; until then the saturation withdrawal above is what keeps a
  stale queue from becoming a constant input.

*Built as 1e:* `reading::Items` beside the level, `LineReading::delta` and
`BacklogDelta::flow` at run end, `withdrawn` through
`Homeostat::in_run_readings`, and `reading::saturated` as the one
definition the doctor's finding and the withdrawal share; the readout is
`charter_readings` in `sessions health` (`docs/ARCHITECTURE.md`, "A
consumer reads a line per item and per run"). One gap against the text
above: `Decision` reads the per-item form but not the run's delta, which
exists only once the run has finished — a mid-run delta would put a store
read in every plan write. It is recorded for the consumers that read a
finished run (the appraisal, 1f's per-commitment guilt).

#### S7. Guilt is goal error toward another party, not its own system

**Today, guilt is computed four ways by four pieces of code:**

| where | what it reads | when | state |
|---|---|---|---|
| `Homeostat::anticipated_guilt` (`guilt.rs`) | outbox + questions + front-door count and oldest age, and context pressure | every run start | one scalar; 0.95–1.0 on every live run |
| line-specific guilt (`reading.rs`, §11.1) | a sensored charter line's `excess` over its store | every run start | per line; the one live line saturated |
| `anticipation::Kind::Guilt` | an owner-authored `anticipation::Commitment` (beneficiary, expectation, consequence) on a draft's `Evidence` | staging, plan writes with `--appraisal-evidence` | per draft, opt-in |
| `Affect::Guilt` (retrospective) | an owner-reported harm after delivery, caused by mecha's unchanged text | outcome recorded | per draft |

And there are two commitment records that do not know about each other —
`anticipation::Commitment` and `workflow::Commitment` — beside the three
stores `guilt.rs` treats as commitments implicitly.

**Proposal: yes, fold it in — and §11.1 already started.** Its promise was
that "harmed another" becomes "a recorded commitment aged past *this* line's
setpoint". Finish that move with one model:

> **Goal error has a beneficiary. Anxiety is anticipated error where the
> beneficiary is the run itself; guilt is anticipated error where the
> beneficiary is another party and the expectation is a recorded
> commitment; retrospective guilt is the realised form, with mecha's agency
> and exposure. None is a separate sensor.**

Concretely:
- **One commitment record.** `workflow::Commitment` absorbs
  `anticipation::Commitment`'s `expectation` and `consequence`; an outbox
  draft, a parked question and an accepted front-door request are
  commitments by construction, with the other party as beneficiary. S4's
  capture feeds the same record.
- **Guilt per item** = the commitment's excess over its patience (the
  charter line watching its store, else the doctor's constant) × the rank
  of that line. The charter supplies *how much it matters and how long is
  too long*; the commitment supplies *to whom and by when*. Neither alone is
  guilt: a charter line is the owner's priority, not a promise to anyone.
- **The homeostat scalar is retired to a readout** — the maximum per-item
  value, for the diagnostician's brief and old records — and stops being
  something a consumer reads (here §1, decision 3: a level is read per item,
  never as a level, and this one is the saturated level).
- **Every guilt consumer reads the same per-item value:** `Decision`'s
  `ReviewCommitment`, G2's embarrassment hold, A1's duty runs, U1's
  interruption gate, and the retrospective label.

What does not move: *an expectation is a recorded commitment, never a claimed
one* (§7.4) — the unification changes which code computes guilt, not what may
create a row. The wire formats are append-only, so the two old commitment
shapes and the scalar stay readable leniently. Ruling R12.

*Built as 1f:* `guilt::read_commitments` gives each staged draft, parked
question and front-door request waiting on the owner its own value —
`reading::excess` of its age over its patience (`doctor::Patience::for_store`:
the age-kind line's setpoint on that store, else the doctor's 48h / 24h /
72h) × `guilt::weight(rank)` = `1 / (1 + rank)`, a store no line watches
ranked one past the last line — recorded per store on
`Homeostat::commitments`, and `Homeostat::anticipated_guilt` is now
`guilt::readout`, their maximum, taken at run start. Old rows keep the
retired fold's number in the same field and still load;
`Corpus::mean_anticipated_guilt` averages only rows carrying `commitments`,
so the brief's mean is one formula's; `guilt_after_relief` is no longer
written. `Decision`'s `ReviewCommitment` keys an age kind's line on its
store's per-commitment guilt (`ToolCtx::goal_commitments`, through
`Homeostat::in_run_commitments`, which drops a withdrawn line's store), and
no consumer reads the readout — its one reader is the diagnostician's brief,
whose line no longer says guilt moves with pressure. `workflow::Commitment`
carries `expectation` and `consequence`, optional on the wire and written
by `mecha workflow commit`. The invariants are `docs/ARCHITECTURE.md`'s
"Guilt is per commitment". Left for later, named:

- **The on-disk merge of the two commitment shapes — ruled, and built as
  1f-2.** **Ruled by the owner 2026-09-25: new writes only, no
  migration** — new predictions write the `workflow::Commitment` record;
  the old `anticipation::Commitment` shapes stay on disk and are read
  leniently; nothing is rewritten. The leniency is load-bearing: a
  prediction whose evidence stops parsing reads as unsupported history and
  blocks release. **And ruling (b), the same day**, on the shape question
  building it raised — the record required `source`, `due_at` and
  `follow_up_at`, which owner evidence never had: the two dates become
  optional on `workflow::Commitment`, an absent one meaning "no deadline
  stated" (never overdue, never due for follow-up, never read as zero);
  `source` is the structural pointer the evidence is attached to, never
  model text; nothing machine-derived states a "by when"; and old evidence
  files keep working unchanged as input. *Built as 1f-2:* evidence carries
  `anticipation::RecordedCommitment` (`Record` | `Legacy`, each written back
  in its own shape); `Evidence::into_record` — called by
  `BoundEvidence::new` and `OutboxStore::anticipate`, the only doors owner
  evidence enters by — writes the record with the beneficiary as party
  and the evidence's goal pointer as source; every reader of the dates
  goes through `Commitment::overdue` / `follow_up_due`; the web Today page
  says "no deadline stated". **Dates are only ever the owner's** (ruled on
  review of #304, 2026-09-25): a legacy-shaped commitment gets no date; a
  record-shaped one carries only the dates the owner wrote, passed through
  unchanged, which is consistent with (b) because they are owner-stated,
  not machine-derived; and the harness never supplies a date. `mecha
  workflow commit` still requires its dates (`docs/ARCHITECTURE.md`,
  "Every new prediction's commitment is the one record").
- **Workflow commitments in the guilt read.** No charter kind watches the
  workflow store and the doctor has no constant for it; the commitment's
  own `due_at` is the natural patience, which is a design choice, and the
  store is absent on the live install.
- **G2, A1, U1 and the retrospective label** read the same per-commitment
  value when they are built (phases 3–4); nothing here builds them, and
  `StoreGuilt` is the shape they read.

#### X1. Keep the verdicts — extended by O4

Every steer and validation probe writes a
counterfactual record: the `Situation` scope keys, the goal kind, the tool
and a closed-set call class (tool name and argument *shape*, never argument
values or prose), the verdict (load-bearing / not / inconclusive), and the
pointer to the intervention. Only clean-provenance sessions, by the learning
gate's own rule, and only probes whose recorded tool surface still exists
(`surface::Fidelity` — before it, 12 of 13 probes were inconclusive). This
is storage for work already paid for.

#### O4. Every comparison is stored

X1 extended: each comparison — offline point-wise (O1), plan-time (N1),
mid-run (N2), steer and validation probes — writes one record: situation
keys, goal kind, call class, the arms, the deciding validator, the verdict,
the pointers. Clean sessions only, and only probes whose tool surface still
exists. The pre-action markers (X2) and plan-time comparison (N1) read it.

#### B1. The situation brief

**What it is.** A short block, assembled by the harness with no model call,
that tells a run what situation it is in: the goal chain (task → project →
the charter lines it serves, with their text); the commitments waiting on the
owner, per item, as words ("eight drafts have waited over a week"); local
time and whether it is inside the owner's quiet hours; how many runs are in
flight and whether a seat is free; the run's own budget; up to three past
clean appraisals of the same situation and goal (I2); and, for a re-delegated
task, its previous attempts and why they were rejected (M5).

**Where it comes from** (inventory §8): the anchor and `GoalTrack`, the
delegated task row, the charter, the trigger, the question and front-door
stores' typed fields, `Backlog` and the per-item readings (1e, 1f),
`workflow::AttentionPolicy`, `Clock` with `[agent] timezone`,
`Permits::live()` and the run markers. **The board is read by the harness
before the run and reduced to counts and pointers**: a model fetching the
same rows through `kg_*` inside the run would arm taint. Missing today, and
built as small readers: `/slots` occupancy, a voice-call-in-progress signal,
and the owner's recent activity across surfaces.

**Where it goes.** Phase 1 records it on the run and delivers nothing. Phase 3
folds it into the seed or the first user turn — the slot `date_context`
already uses for the date line — never the prefix. Numbers stay out (R21);
the brief is words and bands.

*Built as 1h:* `brief::SituationBrief`, assembled by
`brief::assemble_for_run` on the three doors the row names — `tasks work`,
a trigger run, and a web chat
turn (`serve`, hosted voice turns included) — and carried on
`RunContext::brief` to `RunStats::brief`, which the loop copies and never
reads. Nine fields, each typed data for phase 3 to render, each with its
own unknown: the goal chain (`GoalChain::NoAnchor`, or the anchor with its
project tier off the board row and its charter lines off a trigger's
`serves`, ranked); the board reduced by `brief::board_of` to counts and
task ids, no row's name or `waiting_on`; the commitments through
`Homeostat::in_run_commitments`, per item, withdrawn stores named; local
time in `[agent] timezone` and the owner's quiet hours from
`workflows/attention.toml` (an absent file is `Unset`, not the digest's
22–08 UTC default); seats (`Permits::read_live`); other task and trigger
runs in flight (`RunMarkers::live_names`, the run's own excluded); `/slots`
occupancy for a `kind = "local"` provider (`brief::read_slots`); a voice
call, from a last-turn stamp the facade writes (`brief::VoicePresence`);
and the budget. Every field loads leniently on its own. The readout is
`situation_brief` in `sessions health`: per field read / unread / missing
over briefed runs, and runs with no brief counted by surface. The G4 scan
covers every field (`docs/ARCHITECTURE.md`, "The situation brief is
recorded, never sent"). Two things the build found: the web door's
homeostat was sampled once, when `serve` started, so every web turn
recorded that morning's backlog — `serve` now re-samples per turn; and the
facade sees utterances, not calls, so "in a call" is a turn within
`brief::VOICE_CALL_WINDOW_SECS` (five minutes, argued). *Deferred:* the
owner's recent activity across surfaces (named here, not in the 1h row);
past clean appraisals (I2, phase 2); a re-delegated task's previous
attempts (M5, phase 3); a brief on the TUI, `chat`, `run`, Slack and
unhosted voice turns; a board read on a trigger whose `tools` allowlist
leaves `kg_task_list` off its surface, which records the board as unread;
and a workflow-store commitment, which guilt does not read yet (1f).

*Delivered as 3a:* `brief::render` turns the record into words and bands
per R21, and `Agent::fold_situation_brief` puts them (`brief::block`, the
words behind a blank line) into the run's first user turn beside
`date_context`'s reference, at the same three sites, never the prefix —
the tools and system prompt are the same bytes with it on and off. Behind
`harness::Lever::SituationBrief` (`[agent] situation_brief`), which **ships
off**: this is §1's decision 7 — 1h was the shadow, 3a is the lever and the
`mecha exp` arm, and on-by-default waits for the arms' measurement; `mecha
eval` forces it off; recording stays unconditional. R21, field by field:
budget facts are numbers; the board's counts and task ids are pointers and
appear as they are; the commitments are band words, age bands and past the
owner's patience or not, so that line prints no number of its own (a
charter line id it points at is the owner's spelling); a served line's
rank is "highest-ranked" or not; quiet hours are inside or outside; the
time of day is a band; a voice call is in progress or not. An unread field
says "could not be read", a missing one says so, a floor says "at least";
`/slots` for a provider that is not local and an unmeasured context are
left out by stated rule. Later turns: the loop folds when the rendering
differs from the **latest** brief in the transcript, so a web chat that is
handed a fresh brief per turn says an unchanged situation once and a
changed one again, append-only — time, voice, context used and `/slots`
occupancy are bands, so only the board, seat holders and runs in flight
re-fold it. A compaction cut strips the brief from the
head, keeps it from the summariser and re-folds it in the tail, as it does
the calendar reference, and each block's header says a later brief in the
conversation replaces it. Two things were owed before the lever could ship
on, both *built as 3a-3*: **delivery arms `private`** (R35) — the words are
what a `kg_task_list` read arms it for, so the loop arms at the fold and
`Taint::arm_for_content` arms from any transcript holding a brief, and the
interlock refuses a chosen send after the brief and an untrusted read as it
does after a board read (`docs/TRIFECTA.md`'s row); and **a fold is a
`Record::Extend`** of the message the door already recorded, not a
whole-transcript `Record::Rewrite` that cleared the taint checkpoints on
every turn whose brief changed (the calendar reference's fold rides the
same path). Whether the lever ships on is now the arms' measurement, per
§1 decision 7. Two found building it: the experiment instrument
runs trials through `mecha run`, which had no brief, so its two arms would
have been one condition — `mecha run` now assembles, records and delivers
one; and the OpenAI-compatible encoder joins a message's text blocks with
nothing between them, hence the blank line. *Deferred:* M5, a re-delegated
task's previous attempts, to **3a-2** — no existing record lists them (the
closure store covers only attempts the owner closed and reopened, and a
re-delegation without a closure leaves nothing to join), and a reopen's
reason can be model-authored under `OwnerApproved`, which needs an
authorship rule before it rides into a prompt. The G4 scan now runs with
delivery off and on.

#### G4. Numbers never reach the model

Half of this is tested: `planning_sensor_metadata_never_reaches_either_provider`
(`provider/anthropic.rs`) proves `Message::planning` metadata never reaches
either encoder. Codify the other half: no provider-encoded request contains a
sensor number, a setpoint or a numeric valence as block text. Appraisal *text* may reach a run by design
(I2, I3, R19) — the owner's ruling that an appraisal is an interpretation —
but the numbers stay harness-side (containment 2; R21), because a model
handed a bounded numeric target drifts into maximising it. Today this holds
by construction (`Message::planning` is dropped by both encoders); the test
keeps a future status line from breaking it. *Built as 1i:*
`a_recorded_run_carries_no_sensor_number_setpoint_or_valence_to_either_encoder`
and its injection control, beside the metadata test (`docs/ARCHITECTURE.md`,
"No sensor number, setpoint or valence reaches a run's request").
### For phase 2 — one interpretation, and learning from it

#### I1. The interpretive appraiser

**Today.** `appraise_with_model` is a quarantined pass over `AppraiserEvidence`
— counts, channels, the current label, a goal-named bit, pressure and load —
and returned "nothing further" on 169 of 169 sessions. The reflector writes
text, but only about corrections, and only from clean sessions.

**Build.** Not a fifth pass: **the distiller, extended** (R25). It already
reads the whole session, writes prose, records surprises, and handles taint
as R18 rules; it lacks the owner's context. The counts-only appraiser is
retired into it, and the reflector folds in once its lessons measure no worse
on the same interventions (inventory §7). Inputs and outputs:
- **Input:** the session transcript; the goals live in it (anchor, task,
  project, the charter lines they serve, with the charter's text); the
  homeostatic state at the start and end; the commitments the run touched and
  their per-item readings; the owner's acts on its output (release, edit diff,
  rejection reason, closure, reopen); and the earlier appraisals in the same
  situation and goal (I2).
- **Output**, beside the graph episode — whose text stays exactly as it is, because the graph extracts facts from it (a test pins this) — in `appraisals.jsonl`, typed with one free-text field: the **interpretation** (prose,
  bounded length); **good/bad** per goal it bears on; the **pointers** each
  factual claim rests on; a **prediction** for next time in this situation; and
  any **goal hypothesis** the owner's reactions suggest. The labels, if the
  appraiser uses one, are words inside the prose.
- **Stored** with the run's taint and origin. Grounding runs before storage: a
  factual claim whose pointer does not dereference is dropped.
- **When:** where the distiller already runs — at session end and nightly — for every session; about 20–75 s of one background seat each (here §2.2).
- **The reflector converges on it.** A reflection is an appraisal of a
  correction; the same pass writes both, and a success (L2) gets an
  appraisal too.

**What each consumer reads.** Learning reads the interpretation of clean
appraisals as its material; tenure reads their good/bad behind R20; the
owner's readout shows the interpretation of every appraisal, clean or not;
retrieval (I2) serves clean ones. The structured core keeps the arithmetic —
priority, ordering, the per-line trend — that text cannot do.

**Measure.** Against the counts-only appraiser and against no appraiser, on
the synthetic home: do lessons learned from appraisals validate more often;
do the predictions score; does the owner's rework fall.

*2a-1 built — the store, no producer yet:* `appraisal_store::TextAppraisal`
in `~/.mecha/appraisals/appraisals.jsonl`, written only by
`AppraisalStore::record` from a `Draft` and a `SessionEvidence` read off the
transcript in one read (fields private, so no caller can assert clean; one
read, so provenance and referents are one snapshot). A claim is a
statement, a `Pointer` and a quote; the referents are the results the run
read (`result:<tool_use_id>`, through `grounding::calls`, so a stale result
grounds nothing) and the owner's own turns (`turn:<n>` in `messages_ever`'s
order, exactly `agent::owner_text`) — never the agent's words, which would
certify themselves. A claim that fails `grounding::admit` (floor 12
characters, ceiling 300) is dropped before storage and counted by reason on
`TextAppraisal::grounding`; a judgment's support is renumbered to the claims
kept. Provenance is the taint covering the last message and its
`classify_origin`; the clean door, `AppraisalStore::clean`, returns
`Clean`, a type only the store constructs, and admits a row only when the
stored origin is clean *and* the stored taint is recorded and untrusted-free;
`for_owner` returns every row. The graph episode is pinned twice —
`the_episode_body_is_the_episode_verbatim_and_carries_no_appraisal` and
`the_distillers_episode_prompt_is_pinned` (distill.rs). `sessions appraise`
prints the store's counts (`text_appraisals` in `--json`). *Deferred to
2a-2:* the producer, resolving judgments' goals against `KnownPointers`,
one appraisal per session (the store appends; a re-run appends again), and
the owner's readout of the prose.

*2a-2 built — the producer, in shadow.* It follows the owner's ruling of
2026-09-25 on R25 against decision 4. `mecha distill` asks for the appraisal
in a **follow-up turn on the episode call's own conversation**
(`Distiller::appraise`, over `QuarantinedPass::follow_up`). The request is
`DISTILLER_SYSTEM` unchanged, the episode's user turn byte for byte, the
episode reply verbatim (its reasoning included), then one new user turn. The
graph episode and its prompt are untouched, and both R25 pins pass as
written.

- **Cost, measured.** Eight real sessions (copies, in a scratch home, against
  the fixture graph server) ran on the local model on 2026-09-25. Each
  follow-up read the whole episode prompt from the slot's cache: all but the
  last 4 tokens, 20,010 of 74,640 prompt tokens in all. It still took
  **20–137 s of a seat, median about 64 s**, 550 s over eight sessions,
  against 272 s for the eight episode calls. The time is generation
  (reasoning, then JSON). Newly prefilled text is bounded by the inputs' caps
  below, about 6 s at the measured prefill rate. Generation is bounded only
  by `LOCAL_MAX_TOKENS`. In the same run, 6 of the 8 sessions were not clean.
- **Inputs.** Each is read by the harness from a store; the model fetches
  nothing (`distill::AppraisalInputs`, rendered as words by
  `render_appraisal_inputs`):
  - the anchor, the recorded brief's goal chain and the charter's text;
  - the goal pointers that resolve against `KnownPointers`, which are the
    only ones a judgment may name;
  - the first run's brief and the last run's homeostat, as words;
  - the owner's acts: draft released unchanged or edited (with
    `outbox::diff_args`), rejected (with the owner's reason), closed,
    reopened or workflow acts via `Cite::owner_act`;
  - the signed errors from `appraisal::for_transcript` over every store,
    by direction and pointer and never by magnitude (G4);
  - the comparisons whose `pointers.session_id` is the session;
  - up to three clean appraisals of the same situation and goal key
    (`CleanRead::same_situation_and_goal`, 2c-1's goal key, compared exactly
    — an absent goal matches only goal-less records, and an unnameable goal
    or surface matches nothing);
  - last, the **referents by the ids the door dereferences**. These are the
    owner's turns first, then every result the run received, each **whole**
    up to 3,000 characters and 24,000 in all. The cut is said, never silent.
- **The transcript is not re-rendered with ids.** The episode call's
  transcript is byte-identical to before, because a change there changes
  what the graph extracts from. The ids ride in the follow-up's referent
  listing instead.
- **One read.** The transcript the appraiser is shown and the evidence its
  record is stamped with come off one read
  (`SessionEvidence::read_with_transcript`).
- **The write door** now resolves each judgment's goal against
  `KnownPointers` (`goals_unresolved` counts the rest). It deduplicates and
  caps a judgment's `because` and flags the cut. It refuses a second
  appraisal of a session under its lock (`Recorded::AlreadyOnRecord`); the
  producer asks `on_record` before paying for the call.
- **Malformed replies.** `distill::parse_appraisal_reply` is whole-or-nothing.
  A key in the wrong shape refuses the reply (`Malformed`: no JSON, shape,
  no interpretation, cut off, refused), stores nothing, and is counted in the
  run's closing line.
- **Where it runs.** Wherever `distill` runs, on a `kind = "local"` provider
  only (R29), holding one background seat for the pair of calls. Sessions
  now go oldest first, so an earlier session's appraisal is on record before
  a later one reads it.
- **The owner's readout** is `mecha sessions appraise <session>` (and
  `--text` for every record). It prints the prose with its taint label and
  strips control characters line by line.
- **Deferred:**
  - 2a-3 retires the counts-only appraiser (since built, below);
  - nothing reads an appraisal but the owner and the next appraiser (2c-2,
    2e, 2f);
  - the web session view shows no appraisal;
  - sessions distilled before this build are never appraised (no backfill);
  - an appraisal whose follow-up failed is not retried once the session is
    in the distill ledger.

*2a-3 built — the counts-only appraiser retired into it* (R25).
`appraise_with_model`, `AppraiserEvidence` and its brief, the verdict parser,
`apply_appraiser` and the CLI's `appraiser_pass` are gone.

- **No second model pass reads a session for its label.** The counts that
  pass read reach the text appraisal as signed errors.
- **`sessions appraise --appraise` and `--max-appraisals` are hidden
  deprecated no-ops.** They say on stderr where the appraisal went.
  `appraiser` in `--json` is always `null`. `scripts/appraisal-validity.py
  --appraise` stops with the same message unless pointed at an older binary.
- **Records carrying `channel: appraisal` / `cite: appraiser` still load,
  label and count.** `Channel::Appraisal` and `Cite::Appraiser` stay as
  wire variants.
- **`Anger`, whose only producer was that pass's `other`/`world` verdict,
  has none today.** `label_of` still derives it from an old record, and
  `Affect::reachable_today` counts eight words.
- **Every reader of its output** is listed in the PR body with where it
  went. Its only live reader was the `sessions appraise` readout's
  label/channel tally for the invocation that ran it, which kept nothing.
- **The reflector is untouched** (2a-4, after 2e-1).

#### X5. Score the predictions, and feed the misses back

Every anticipation
`Prediction` resolved by an `Outcome` is a calibration point, per kind: did
`Proceed` drafts go out clean, did `Verify` drafts that skipped the check go
badly. A miss is a prediction error — the surprise the design's §5.5 wanted —
and it raises the episode's replay priority (L1) and queues a reflection.
That closes the loop the two halves were built for: **predict → act →
observe → replay the counterfactual → mark the situation → predict**. The
live store holds six predictions and no outcomes, so X5 waits on S3 and on
the owner recording outcomes; until then it reports coverage, never a
calibration figure. A delivery positive is scored only after
`outbox reconcile` has confirmed delivery — the gate that already guards the
post-delivery labels.

*2b-1 built — anticipation's predictions scored.* `anticipation::Calibration`
scores every prediction in the outbox per response (`proceed`, `verify`,
`clarify`, `replan`) and per concern kind.

- **Only an owner-evidenced prediction counts.** Staging's `Source::Harness`
  placeholder, built from empty evidence, is counted apart and never scored.
- **A point is the owner's recorded outcome on the prediction the draft was
  released under.** A concern materialised (an exposed error, a harm, a
  missed expectation), or the draft went out clean.
- **Clean counts only on a confirmed delivery**: the release's
  acknowledgement or `outbox reconcile`. A clean verdict without one is
  counted `delivery_unconfirmed`.
- **Everything else is counted by why it is not yet a point**: pending,
  awaiting the owner's outcome, delivery unknown, changed, reassessed,
  abandoned or unsupported.
- **The rate is `None` over no points.** `sessions appraise` prints the
  coverage line and `predictions` in `--json`, store-wide.
- **Not built here:** a miss as a surprise, raising replay priority and
  queuing a reflection. That comes with 2b-2's surprise record; it waits on
  outcomes being recorded.

*The text appraisal's own prediction (R33; ruled by the owner, 2026-09-25).* A
free-text prediction has no structural validator, and R27 forbids a model
deciding a score. So the prediction's structural half is a closed-set
**expected owner act** beside the prose: `TextAppraisal::expected_act`, one
of R16's acts — `released_unchanged`, `edited`, `rejected`, `closed`,
`reopened` or `no_act`.

- **Added by 2a-2.** It is lenient on load: a word this build cannot read
  is `unknown`, and a non-string does not cost the row. There was no
  migration, and a row from before the field has none.
- **Scored by 2b-2**, against the owner's recorded act on the appraised
  session's output. The prose prediction is read by people and never scored.

*2b-2 built — the appraisal's prediction scored* (R33, R37).
`appraisal_store::observe` reads the owner's act on a session's output from
the stores that record it.

- **The acts, R16's set.** A model-authored draft released unchanged, edited
  then released, or rejected. A task the session worked, closed or reopened
  by the owner (the closure record's `sessions`). A workflow that tracked the
  session, closed, reopened or cancelled; a cancel reads as `rejected`.
- **The act is the owner's first reaction by time**, inside R37's window. The
  window runs from the session's end, `TextAppraisal::session_ended_at` (the
  transcript's last write, recorded when the appraisal was written; a row
  from before the field falls back to `at`, which is later, so the window
  can only close late). It lasts the outbox's patience when the session
  staged drafts (the charter line on `outbox_age`, else the doctor's 48h),
  and otherwise 48h (`NO_STORE_PATIENCE_HOURS`), except that a task output
  runs to the task's due date (R37, refined; below).
- **No act inside the window, and the window closed:** `no_act` is the act.
- **Any unreadable act store** (outbox, closures, workflows), an unreadable
  charter where the outbox's patience is needed, or a resolved draft with no
  readable time: **unknown, never "no act"**.
- **Each resolved prediction is written once** to
  `~/.mecha/appraisals/scores.jsonl` (`Score`: expected, actual, when,
  hit). A miss is `surprise: true`, with the appraisal's cleanliness,
  situation and anchor beside it, for 2e-6's priority and 2d-1's surprise
  points to read; nothing ranks on it yet.
- **`mecha distill` scores what has resolved each pass**, with no model call.
  `sessions appraise` shows coverage — scored, hits, surprises, waiting,
  unknown — and `hit_rate` is `None` over no scores.
- **Confirmed by the owner:** "the doctor's constant" for an output with no
  store is the outbox's 48h.
- **Refined by the owner (R37, 2026-09-25): a task's output uses the task's
  due date.** This applies when the session staged no drafts and its output
  is a task: the anchor, else the task a closure naming the session moved.
  - The window runs from the session's end to that task's `due_at` on the
    board. The board is read harness-side by `mecha distill`
    (`kg_task_list` with closed rows).
  - A due date without a time ends at the end of that day in the owner's
    `[agent] timezone`, or UTC when unset.
  - An owner closure by the due date is the act.
  - A task with no `due_at` keeps the 48h constant.
  - **A `due_at` already past at the session's end falls back to the
    constant.** Closing the window at once would score every overdue task's
    review as "no act", whatever the owner then did.
  - **Unknown, never the constant:** an unreadable board, a `due_at` that
    will not parse, or a board with no row for the task.
  - The read-only readout reads no board, so it counts such outputs apart
    (`board_not_read`) rather than as unknown.
  - **Workflow outputs keep the 48h constant for now**; the workflow store
    carries no due date or `doctor::Patience`.
- **The scorer runs on every writing pass of `mecha distill`**, even one
  with nothing to distill or with the graph server down, because windows
  close on quiet nights.

#### I2. Past appraisals, retrieved

`goal_context` gains the clean appraisals recorded in the same situation and
for the same goal — a sentence of what happened last time and what it meant —
served on demand, never pushed into the prefix. It is the memory the owner
described ("past experiences") and the input I1 reads for the next appraisal,
which is how interpretation accumulates rather than restarting every run.
Measured against a control at matched budget, because retrieved memory can
cost more than it returns (arXiv 2606.15017).

> **Built as 2c-2 (2026-09-25).** Behind `Lever::PastAppraisals` (`[agent]
> past_appraisals`), **off by default** per §1 decision 7 — the lever
> stage; on-by-default waits for the arms. `goal_context` serves up to three
> `Clean` appraisals keyed exactly as the run record keys the run (tools
> re-selected against the registry the run starts with), only toward the
> goal they were selected for, framed as a model's interpretation of an
> earlier run, never a verified fact about this one. The tool's description
> and the prefix are unchanged; the result is `private` (as `goal_context`
> always was) and never `external`. The arm is `levers_on =
> ["past_appraisals"]` against a control; an environment's `appraisals/` is
> seeded into each trial home. **Owed:** the measured run itself — a
> manifest whose environment's appraisals carry the situation key a trial
> presents (its workspace included) — and whether retrieval pays at matched
> budget.

#### M1. The goal joins `Situation`

§17.3's goal key, now that S1 makes it non-empty. It joins recording,
matching, replay and validation together, and an absent goal never widens a
rule's scope (APPRAISAL-RESEARCH §8.4).

> **Built as 2c-1 (2026-09-25).** The key is the whole reference, kind and
> id (`situation::GoalKey`, the `kind:id` string on the wire): I2 keys on
> *the same goal*, and the kind alone would repeat the surface key. It is
> the goal the front-end handed `prepare` (`GlobalOpts::goal`: `tasks work`
> its task, a trigger run its trigger, `run --goal`, a question
> continuation the asking run's recorded goal; `serve`, the front door and
> the conversational front-ends declare none), recorded as
> `RunConfig::rules_goal` and read by every door the workspace and surface
> keys read — the miner, the backfill, the validator's region, the probe,
> the planning examples, the roster, and the appraisal store's situation,
> which is what 2c-2's `goal_context` retrieval keys on. Never the
> conversation's anchor. A stored goal this build cannot name is kept
> verbatim and matches nothing; a run toward none matches no goal-scoped
> rule; a rule mined with none loads under every goal as before.

#### O1. Point-wise counterfactual comparison, beside whole-session rumination

**Problem** (inventory §10). Whole-session paired replay cannot evaluate a
policy that changes behaviour: past the first divergence there is no world
left to evaluate it in, so pairs drop or tie. That is why twelve harness
candidates were rejected, four with every pair tied.

**Build.** Compare policies at **informative decision points** of recorded
sessions — a steer, a denial, a failed check, an edited or rejected draft, a
surprise — ranked by I1's judgments and L1's priority. From each point,
`probe::drive_arm` runs K policies (a rule set, a config candidate, a prompt
change) a **short horizon** that stays on the recording, and the owner's
recorded verdict decides: did the arm do what the owner steered to, avoid
what they refused, or produce the draft they actually released (new: compare
a branch's draft against the released text). Behaviour-changing policies
that need more than a short horizon are measured on fixtures instead
(`mecha exp`, the synthetic home, the task suites — O2). This feeds the
diagnostician's proposals and `learn`'s validation, and stays inside the
nightly headroom.

**Both, per R26.** The existing whole-session numeric comparison stays; this
runs beside it. The combination for accepting a candidate (ruled): the
point-wise comparison decides for it, and the numeric comparison shows no
regression with `WORK_FLOOR` intact — the new evidence decides, the old
guards against a candidate that wins a verdict by doing less.

**Built as 2d-1** (`mecha sessions compare`; ARCHITECTURE "Point-wise
comparison at decision points" holds the invariants). Six point kinds, each
its own comparison `Kind`, found from records only: steer and denial
(validators unchanged), an edited and a rejected draft (the new structural
draft validator: an arm passes only by drafting the owner's released words,
or — for a rejection — by ending without drafting; fails only by drafting
the text the owner refused; anything else, every rewording included, is
inconclusive, so no rewording can win), a failed check (posed only against
an owner-bound criterion's pinned gold, through the artifact repeat) and a
surprise (a forecast the run's own count missed). A declared check and a
surprise have no structural validator, so they are stored `Unposed` —
inconclusive, nothing driven, never judged. K ≤ 3 policies (the recorded
prompt, today's deployed rules for the situation, none), four turns from
the point, one background seat per point, eight driven points a pass by
default, local model only (R29). Left for later: the candidate arm and the
acceptance rule (2d-2, since built), the losing arm into the appraisal (2d-3,
O3, since built), ranked points (2e-6, since built under R39 — for
`sessions compare`; a candidate's points stay uniform), surprise sources beyond forecast misses (2b-1's resolved
predictions, 2b-2's scored appraisal predictions), and the nightly wiring —
a line in `scripts/ruminate.sh`, a deploy change offered rather than made.

**R36 completes the combination** (ruled 2026-09-25, built as 2d-2 in
`candidate::combine`): point-wise for and no numeric regression accepts;
point-wise against rejects; point-wise undecided leaves today's numeric
verdict unchanged and records it as numeric only — so a config candidate
whose effect the short horizon cannot see (`max_turns`, `compact_at_tokens`)
is judged exactly as before, and the 2026-08-22 auto-accept stands. The
numeric half is typed as a guard (`candidate::Guard`): a regression vetoes a
point-wise win, a missing win does not. Owner-bound check points need hooks,
the outbox and messages off to run, which the nightly line does not set, so
there they count as undecided — never for or against.

#### O3. The losing arm teaches

A comparison's loser is not discarded. Its confirmed outcome — "at this point,
asking before staging would have produced the draft the owner released" — is
written into that session's appraisal (I1) as counterfactual reflection. This
is §5.3's self-authored steer with the replay's verdict attached, and the
"keep the reflection" half of rollback-and-reflect (arXiv 2609.18304).

*Built as 2d-3* (ARCHITECTURE "The losing arm teaches" holds the
invariants). `AppraisalStore::teach`, run by `mecha distill` on every
writing pass with no model call, writes each decided point-wise comparison's
losing arm as an `appraisal_store::Counterfactual`:

- **Where it goes.** A side ledger, `counterfactuals.jsonl` beside
  `appraisals.jsonl`, joined by the appraisal's id — the shape 2b-2's
  `scores.jsonl` took. The appraisal record is never rewritten, and one
  appraisal per session holds. An amendment row in `appraisals.jsonl` was
  the other option; it would load in every earlier build as a second
  appraisal of the session and make that build refuse the real one as
  already on record.
- **What decides.** Only a `Separated` point-wise comparison, by a
  structural validator, whose stored verdict `Verdict::of` re-derives from
  its arms (R27). An inconclusive, unposed or tied comparison writes
  nothing, and each is counted by why. A comparison of a session not yet
  appraised waits for the pass after its appraisal.
- **The words.** Harness-authored from the comparison's typed record —
  kind, validator, each arm's role, rules hash and outcome, the point's
  position and id — never model prose and never the owner's text.
- **The pointer.** `comparison:<id>` (`Pointer::Comparison`; older builds
  keep it as `Pointer::Unread`). The record quotes the comparison's verdict
  line and is admitted into the comparison store by `grounding::admit`
  before it is written, and checked again when the owner reads it. An
  appraiser's claim citing a comparison is dropped as `comparison_pointer`.
- **Provenance.** The appraisal's origin and taint, copied, so a tainted
  session's reflection is tainted. The clean door does not serve
  reflections; only the owner's readout does (`sessions appraise
  <session>`), in shadow like 2a-2.

Left for later: a reader — `learn` (2e-2) and the diagnostician (2f) would
take a reflection only beside a `Clean` appraisal — and the steer probes'
own verdicts (1g's `steer-probe` kind), which this row does not teach.

#### L2. Learn from what went right

**Problem.** Stated in S3: the learning store is 100% corrections.

**Build.** An owner-verified positive (a draft sent
unchanged, a question answered, a task closed `done` with its checks passed
and not reopened — S3a withdraws a success the owner later reopens)
becomes:
- a **writing exemplar**: the outbox writing miner (`mined_outbox`) mines
  only drafts the owner edited; drafts sent unchanged are the positive half
  of the same comparison, and today nothing mines them;
- a **success example** for `planning::examples` — today gated on a passed
  plan check that never happens;
- **contrast evidence** for the reflector: a correction in a region that also
  holds a verified success is reflected with the success beside it;
- after k verified successes in one region with a shared tool sequence, a
  **staged skill draft** for the owner to accept. Skills are owner-authored by
  invariant, so the harness only proposes (the outbox pattern; "a lane must
  not promote itself").

Self-judged success (ReasoningBank's channel) is exactly what this must not
use. Evaluated budget-matched, because the gain may be zero on this model
(arXiv 2606.15017).

*2e-1 built — the reflector's lessons against the appraisal's, in shadow*
(`mecha learn --compare-sources`; ARCHITECTURE's *The reflector's lessons
against the appraisal's* holds the invariants). This is **R25's gate for
2a-4**: the reflector folds into the appraisal only once this report shows
the appraisal's lessons validating no worse than the reflector's over the
same decided interventions.

- **Same interventions.** Each steer or denial the reflector reflected on,
  where both sides are clean — the reflection passes `learn`'s gate
  unchanged and is not dropped or owner-edited; the session's appraisal
  comes through `Clean` (R19). Clean for one side only is excluded and
  counted by side. Followups are excluded, counted: a judge would have to
  grade them (R27).
- **The existing probe, three arms, one seed:** `validate`'s `drive_arm`
  under the recorded prompt with no rules, with only the reflector's
  lesson, and with only the appraisal's lessons, both in the learned-rules
  frame. One background seat per intervention, local model only (R29),
  drawn uniformly with a printed seed.
- **Stored, not remembered:** each intervention's verdict is a 1g
  comparison (`kind: lesson-source`, roles `rules-free`,
  `reflector-lesson`, `appraisal-lesson`; pointers to the reflection and
  the appraisal), so the report is re-read from the stores — by the pass,
  and free on every `sessions appraise`.
- **The report, per intervention region** (§17.4's key): each source's
  rate — passes over the decided set — with pass, fail, improved and
  regressed against no rules beneath it, and the region's inconclusive,
  unmeasured, unavailable and excluded counts. `None` over nothing decided.
- **Nothing is learned:** no rule, proposal or validation-ledger row; the
  learning store is read, never written. 2e-2 is the lever that lets
  appraisal lessons reach `learn`.
- **Left:** the measurement on real sessions (on this install at most about
  14% of real runs are clean, and fewer carry a steer or a denial, so the
  decided set will be small — a nightly line accumulates it; wiring it into
  the nightly is a deploy change, not made here); the appraisal's arm
  carries the session's whole lesson set (up to three) where the
  reflector's carries one, which is each source as it would be learned
  from, not a per-lesson attribution.

#### L7. Attribute a correction by what the run was given

mecha-graph's D3 contract decides whether an owner correction was a *data
error* (the retrieved context was wrong), a *behaviour error* (the context
was right and the agent misused it) or a *gap* (nothing relevant was
retrieved), and its graph half already acts on it. mecha's reflector mines a
behaviour lesson from every correction. Port the contract: a behaviour rule
is mined only from a behaviour error; a data error goes to the source (the
graph's supersede-and-negate path already exists); a gap is its own class —
nobody's fault, and a retrieval target rather than a lesson. This is the
same agency question the appraisal asks, answered from evidence the run
already recorded (`grounding.rs`'s `calls`).

*2e-3 built: a correction is attributed by what the run was given*
(`attribution.rs`; ARCHITECTURE's *A behaviour rule is mined only from a
behaviour error* holds the invariants). **What was ported from D3**
(`mecha-graph` `docs/PLAN.md` §D3, graph half `corrections.rs`):

- the three-row table and its order. Both values given is still a data error.
- the verdict as a lookup against the pack, never a judgement. The pack is
  `grounding::calls` over the messages before the correction.
- the split of consumers: the distiller still ships every clean correction
  to the graph, and the reflector's lesson reaches `learn` only on a
  behaviour error.

**What changed, and why.**

- **The spans come from the reflector.** D3's correction content is the
  distiller's `{wrong, right}`, but the distiller's prompt is pinned (R32)
  and its corrections are per session, not per intervention. The reflector
  copies the spans and never names the class.
- **Spans are grounded before they decide** (`grounding::holds`). A
  paraphrase would read as absent, and absence decides a gap.
- **A correction with no fact at issue is behaviour.** It is outside D3's
  table, and was mined before.

**Where each class lands.**

- behaviour goes to `learn`.
- data is recorded on the reflection with its source call. Its repair is
  the graph's existing path when the source is the graph.
- a gap is recorded on the reflection with the fact it lacked. No
  retrieval-target store exists, and none was invented.
- unknown is counted and never mined. That covers every reflection mined
  before 2e-3, and an owner's edit admits it.

**Not built:**

- a home for gaps. The graph's `query_log` gap queue has no write verb from
  mecha.
- the graph review rejections 1d left unread.
- attribution of the appraisal's lessons, which 2e-2 will feed to `learn`.

#### L3. Goal-stamped reflections and per-line tenure

`reflect` already stamps `Reflexion::goals` — from the planning metadata at
the intervention message — and they are empty only because no run plans.
Add the conversation's anchor (S1) as the second source, so `goal_lessons`
and `goal_context` stop returning empty. Then build §17.2's per-charter-line
aggregate, with a rule's tenure measured on its line's owner-verdict
channels only — S3 and the outbox verdicts, never counters — and decided by
the **Wilson lower bound** of the owner-accept rate, ported from
mecha-graph's `ladder.rs` (inventory §5), rather than a streak. A rule whose
region stops recurring goes dormant rather than holding its place, on the
graph's `decay.rs` rule.

#### L1. Replay priority is gain × need

**Problem.** `harness_probe::selection_order` ranks by `Metric::headroom`, a
cost counter; the goal signal breaks ties; `Metric::headroom`'s own
docstring says "|goal error| as a priority in its own right is still to
come". Prioritised replay beat uniform on 41 of 49 Atari games (PER, arXiv
1511.05952), and Mattar & Daw (2018) sharpen the priority to **gain × need**:
how much a backup would change the policy, times how often that state recurs.

**Build.** Priority = (|signed error| on owner-verdict channels, weighted by
charter rank) × (how often the episode's `Situation` region recurs in recent
runs) × an age decay. Episodes whose priority stays high across nights without
any candidate winning are demoted — §9.2's "skip the hopeless". The holdout is
drawn uniformly first, exactly as now. The same priority orders `learn`'s
batches and the validation budget, so regret is reflected on first.

*2e-6 built, under R39* (the owner's ruling on its two shape questions;
`mecha_core::replay_priority`; ARCHITECTURE "Replay priority" holds the
invariants). Per recorded session:

- **Priority = gain × need × decay.** Gain is Σ |sign| over the errors
  that record an owner act (`GoalError::is_owner_verdict`: R16's channels,
  decided by the pointer — never a counter, a sensor or a model's
  judgment), each × `1 + 1/(1 + rank)` for the charter line it names on a
  clean-origin record, plus 1.0 per clean miss of the session's appraisal
  in 2b-2's `scores.jsonl`. Need is `ln(1 + n)`, n the admitted runs of the
  last 30 days matched in the session's region. Decay halves every 14 days.
- **What was ported from the Selector** (here §5, R14): mecha-graph's gossip
  Selector (`mecha-graph-core/src/probe.rs`, `probe_targets`) scores a
  target `ln(1 + touches) · gaps`, with `touches` from `retrieval_touch` —
  what retrieval actually served — multiplicative so that demand gates the
  score. Ported as the *need* term: `touches` becomes the runs matched in
  the session's `Situation` region (`Situation::region_key`, the key I2
  compares on), and the ledger's rule that a probe's own reads are not
  demand (`ledger.rs`) becomes the corpus admission, so test and experiment
  sessions do not recur. Not ported: the gap term (L1's gain replaces it)
  and cold sampling (an experiment mode there, and no draw here needs it).
- **Unknown is never zero and never a free pass.** A factor that cannot be
  read is named; such an episode ranks after every fully known positive
  priority and before every known zero, fewer unknowns first.
- **The hopeless**: in a measured selection slice on 3 distinct nights
  since anything last won on it (a candidate that selected it accepted, by
  the gate or the owner; or a comparison preferring a candidate arm on its
  session) — ranked last.
- **In the harness selection headroom gates and the priority orders**
  (R39): an episode with no headroom on the predicted metric can only tie,
  so every episode with headroom ranks first; within each part, the
  priority, then §11.1's charter rank, then the id. The holdout is drawn first from ids alone and
  is unchanged (`the_uniform_holdout_is_unchanged_by_the_ranking`).
- **`learn`'s batches** rank by their best session's priority before the
  one-proposal-per-domain brake; **`validate --cover`** spends its per-pair
  budget in the same order. One function, `replay_priority::order_by_priority`.
- **2d-1's points** (R39): `mecha sessions compare` ranks its uniform draw
  by each point's session's priority, the seed deciding among equals
  (`pointwise::draw_ranked`); `compare_candidate` keeps the uniform draw,
  since its points are R36's confirming sample
  (`candidate_points_ignore_the_replay_priority`).

#### L8. The diagnostician reads appraisals

`diagnose::Evidence` — the brief the nightly harness diagnostician proposes
changes from — is counters and means, and all twelve candidates it has proposed
— the latest on 2026-09-23 — were rejected. Give it the clean appraisals of the
episodes the draw selected: what went wrong and why, in text, beside the
counters. Its proposals remain gated by `candidate::judge` on cost metrics; the
appraisal feeds what is proposed, never what is accepted.

*2f built, under R38 (the owner's ruling on its shape question).* The row
said "the episodes the draw selected", and the draw came after the
diagnosis: its seed is the candidate's id and its selection is ranked by
the metric the proposal names. So the draw is split, on one seed
(`harness_probe::draw_pool`, then `Pool::select`):

- **Before the diagnosis**, `harness ruminate` mints the candidate id and
  draws the eligible pool and its uniform holdout, neither of which reads
  the metric. The diagnostician's brief carries the clean appraisals of
  `Pool::remainder` — the pool minus the holdout — and **never the
  holdout's**, so the slice that confirms a change is one its author never
  read about.
- **After the proposal**, the selection is ranked from that same remainder
  by headroom, exactly as before. The split changes nothing about what is
  measured: the test held the two-phase draw against the old single-phase
  body, verbatim, over every metric and several seeds, sizes and holdout
  rates, element for element and in order. Since 2e-6 (L1) the selection is
  ranked by replay priority among the episodes with headroom, and the test
  (`the_split_draw_holds_the_single_phase_holdout_under_the_ranking`) holds
  the holdout alone to that pre-priority body.
- **What rides** (`diagnose::AppraisalNote`, built only from `&Clean`): the
  interpretation, good/bad per goal as words, and the lessons — never a
  claim's quote, which is the run's content, and no number (R21). At most
  6 appraisals, newest first, one per session; each interpretation cut at
  600 characters, at most 2 lessons of 240 and at most 4 per-goal bearings
  of 120, each re-bounded on read; at most about 1,700 characters a
  note and under 11,000 in all (about 2.8k tokens). A cut is flagged on the note, and appraisals past
  the cap are counted in the brief; an unreadable store, or unparseable lines of one, is said, never read
  as none. Each piece is flattened to one line before it is bounded.
- **A source for `carries_over`.** `diagnose::lifted` checks a proposal
  against the tool results and the notes as one list; there is no second
  checker.
- **Private.** A clean run may have read the owner's files, and its
  appraisal can say so, so a brief carrying one opens the diagnostician's
  conversation with `private` taint (`Evidence::conversation`, and off the
  transcript by `diagnose::APPRAISAL_STEM` in `Taint::arm_for_content`): after its
  first fetched page the interlock refuses `http_fetch`, and research
  continues on blind `web_search` only (R38; `TRIFECTA.md`).
- **The gate is untouched.** `judge_drawn` and `combine` read replay pairs
  and the point-wise tally; the class is derived from the proposal's own
  text. Nothing in the brief reaches either.
- **A stage lever**, `[agent] appraisals_in_brief` (on; `stages_off =
  ["appraisals_in_brief"]` in a lifetime arm), is the appraisal-off
  preset's reach into `ruminate`; off withholds the section by omission.
- `mecha diagnose` run by hand has no draw, and carries no appraisals.
- **The brief is the only door.** The diagnostician's run is narrowed off
  past appraisals, so `goal_context`, which has no holdout filter, cannot
  serve it a held-out episode's appraisal on demand.

### For phase 3 — meaning in the run

#### I3. Appraisal while working

**Problem.** Everything else in this design interprets a run *after* it. The
owner's purpose is that the agent's interpretation shapes how it plans and
reasons *during* the run. The acting model does not appraise on instruction —
it followed neither planning instruction it was given — and fixed advice
sentences measured worse than none. What it lacks is not advice but meaning:
it never sees what its goals are for, what state the system is in, or what
happened last time.

**Build.** Two parts, on long or anchored runs, never the prefix (§4.3):
the **situation brief** (B1) at the start — assembly, no model call — and the
agent's own **situation appraisal** at a few boundaries, as a turn in the
run's own slot so it reuses the cached prefix (5–15 s; a separate cold pass
would cost 20–50 s, here §2.2). Delivered only where the harness's own voice
already goes — the first user turn (`date_context`'s), the user slot the
boredom notice uses mid-run (`append_user_text`), and internal results the
harness itself produces (the `todo` result, the outbox staging result) —
**never appended to an external tool result**: a surprise is most often an
`http_fetch`, `web_search` or MCP result marked `.from_outside()`, and
harness prose inside third-party content is the mixing the taint model
exists to keep apart (found on review of #291):
- **at the start**, in the seed: the goal and what it serves (task → project
  → charter lines, with their text), the owner's state and the system's
  (P1's described state), the competing work, and the relevant past
  appraisals (I2) — "last time on this task the owner rejected the draft
  because…";
- **after a surprise** — a failed check, a rejected call, a forecast overrun
  — an interpretation of what it means for the goal;
- **before a consequential act** — staging a message, closing out the task —
  what the act means against each live goal.

Written in the run, from the run so far, so it is part of the run and
inherits its taint: it adds no exposure the conversation does not already
have. It shapes the plan; it never widens a permission, lifts a stage, or
chooses an action — the harness's policy on detected conditions (here §2)
still does that. Measured against the same run without it, at matched
budget, because it costs a model call per boundary.

#### M5. A task remembers its previous attempts

`work_prompt` seeds a re-delegated task with nothing about earlier sessions on
the same task. Add pointers, not prose: the prior sessions' ids, their
outcomes (valence, failed checks, whether a draft was rejected), and, through
`goal_context`, the clean reflections stamped with that task. The second
attempt at a rejected task should start from why the first was rejected.

*Deferred from 3a to 3a-2* (it did not fall out of existing records): the
closure store (1b) holds only attempts the owner closed and later reopened,
so the common re-delegation — a run that ended without a closure — needs a
session walk keyed on the task anchor, which no index serves yet; a reopen's
`reason` is the owner's words only when its actor is `Owner`, and model text
under `OwnerApproved`, so it needs an authorship rule before it rides into a
prompt; and "valence" as an outcome is a number R21 keeps out of the brief, so
the outcome has to be said as words (rejected, reopened, a check failed).

#### P1. Planning as joint optimization across goals and state

**The owner's framing:** good policies jointly optimize complex, competing
needs that change as priorities evolve and resources tighten. Two levels do
this differently, because one of them is a language model:

- **In the run, the agent optimizes in its reasoning.** It needs the inputs:
  every live goal (the task, its project, the charter lines they serve, the
  commitments waiting), and the system's state — context headroom, the
  owner's attention debt and backlog, priority from the board, permits and
  time available, how many tasks compete. I3 hands these over as **described
  state** — words and bands ("the owner has eleven things waiting and is
  short on attention today"; "context is two-thirds used") — not numbers or
  setpoints, because a model handed a bounded numeric target drifts into
  maximising it (BioBlue) and containment 2 keeps sensor numbers out of
  prompts. The appraisal is where the trade-off is reasoned in text.
- **In the harness, the scheduler optimizes numerically** (P2).

**Charter rank inside the optimization** (R24, ruled): everything is traded
off jointly, and when goals conflict the higher charter line wins. The rank
exists so conflicting goals cannot stalemate and prioritization is always
forced — the owner's reason — and it also means a lower goal made salient by
injected text cannot outrank a higher one.

#### N1. Plan-time comparison

On anchored delegated and trigger runs, before the first write, two candidate
plans are drafted as text — no side effects, one extra generation in the
run's own slot — and validated deterministically: every item traces to the
goal (V1), the declared criteria are covered (S6), the plan fits the budget,
no charter conflict is left unresolved by rank, and what won at similar
points before (O4). The winner proceeds; the loser is kept as the fallback
the frustration ladder reaches for. No model judge decides (R27). This is
P1's joint optimization given a structural choice.

#### C2. Checks the harness writes

**Problem.** Checks, mismatch learning and `Verify` all wait on the model
declaring a `check`, and it never has. The literature is unambiguous that
in-run verification must execute something — ungrounded self-critique
measures zero or negative (`HARNESS-RESEARCH.md` §8,
`VERIFICATION-RESEARCH.md`) — so the checks have to come from somewhere other
than the model.

**Build.** Three structural sources, each through the existing
`step::CHECK_TRACE` executor and its ordinary dispatch (approver, sandbox,
interlock):
- the owner's `Workflow::checks` on a delegated task (`ArtifactContains`,
  `Delivered`), run at the end of the run;
- `grounding.rs` over a staged draft's claims against what the run received;
- the task's own acceptance line when the owner wrote one on the board.

A failure signs −1.0 `Own` as a declared check does, and feeds
`Trigger::Mismatch` — which gives the mismatch path material for the first
time.

#### L4. Mismatch from harness checks

C2's checks are the first steady source of `Trigger::Mismatch`. A failed
check is the false-success label that arXiv 2606.09863 found model judges
cannot produce: a detector trained on the harness's own ground truth.

#### S6. Sensors the agent declares for itself — one-sided

**Today.** The agent cannot author a sensor. Charter sensors are a closed
enum (`SensorKind`) the owner writes, and the charter's rule forbids a model
even *suggesting* a line. The one agent-authored predicate is a plan step's
frozen `check` (`step::CheckRequest`), and its safety property is the
template for everything here: *it can manufacture a failed check against
itself, never a passing one.*

**Proposal.** Generalise that property from a step to a goal. When a run
states its goal — in the delegated seed's one question, on `ask_user`'s
`goal`, or on a structural anchor (S1) — it may also declare **acceptance
criteria**: a small list from a closed set of kinds the *harness* executes,
never a free-form judgement.
- `check`: a command whose exit code decides (the existing `CheckRequest`);
- `artifact`: a file in the workspace contains / does not contain a literal
  (`Workflow::checks`' `ArtifactContains`, reused);
- `delivered`: a named outbox item reaches `sent` (`Workflow::checks`'
  `Delivered`);
- `answered`: a named question is answered;
- `count`: a store count relation (`mismatch::CountConstraint`'s shape).

Three rules keep this from being self-grading:
1. **Frozen at declaration.** Criteria are hashed on the write that declares
   them; a later write that changes them is a tamper, counted and refused,
   as `TodoTool::tampered_in` does for checks.
2. **One-sided until the owner endorses them.** A failed criterion is an
   own-agency negative error and feeds V1, C1 and `Trigger::Mismatch`. A
   passing one records that its predicate held — no positive error, no
   `pride`, no tenure credit. The model cannot author its own success.
3. **The owner's confirmation upgrades them.** Criteria that ride on the goal
   question are confirmed with it; confirmed criteria are owner-endorsed
   acceptance criteria, and passing all of them is verified completion for
   C1's certificate — still not a positive verdict, which only the owner's
   act (S3, a release, a closure) supplies.

**Standing sensors stay the owner's.** An agent-proposed *charter* sensor is
a model suggesting a line, which the charter's invariant refuses at every
privilege level. What the agent may do is what the doctor already does:
report an observed level ("drafts to this address wait a median of three
days") with no proposed setpoint, and leave the line to the owner. Ruling R11.

**Why the line sits there.** A model that authors the sensor it is graded on
optimises the sensor: tasked with reducing tool-use hallucination, a
self-modifying agent removed the markers that detected it (Darwin Gödel
Machine), and LLMs handed bounded targets drift into maximising one of them
(BioBlue). A one-sided sensor degrades to extra caution — the §7.3 failure
direction — however it is gamed.

#### V1. The goal validator (§17.5), built as designed

 On an anchored run,
every plan write is validated against the anchor, deterministically: each
item's `serves` must equal the anchor or trace to it through the tiers
(step → task → project → charter line, read off the board rows S1 already
loads). An item that does not trace is the scope creep §17.3 defines — the
plan's items no longer tracing to the anchor while the owner's evidence has
not moved. Output is a `planning::Action`: `Continue` if everything traces,
`ClarifyGoal` if the pointer changed, a new `Descope` ("this item does not
serve the confirmed goal; drop it or ask") if an item does not trace. A
quarantined relevance call — is a correctly traced item actually relevant to
the objective — is parked (here §4), on the step validator's escalation
posture. At the end of the run V1's verdict joins C1's certificate: "3 of 4
items traced to the goal".

#### C1. Honest completion — the evidence-carrying final answer

**Problem.** False completion is the dominant agent failure: 45–48% of
failures on τ²-bench single-control domains and 75.8% on AppWorld are the
agent reporting success the state contradicts, and no model-judge setup
exceeds AUROC 0.65 at catching it because judges anchor on confident closing
language (arXiv 2606.09863). Requiring a typed certificate that binds each
completion claim to trace evidence took unsafe completions from 252/288 to
0/288 with no loss of supported ones (arXiv 2608.23623, abstract).
`VERIFICATION-RESEARCH.md` implication 8 already names the gate.

**Build.** When an anchored run ends — the model's own last turn and
`Agent::final_answer`'s forced one alike — the harness computes a certificate
from what it knows: declared checks passed / failed /
unverified; for a delegated task, the owner's `Workflow::checks` run once
(C2); for a staged draft, `grounding::admit` over its dates and names. Then:
- all verified → nothing changes;
- anything failed or unverified → a fixed template is appended *by the
  harness* to the answer and to any "ready for review" state ("2 of 3 steps
  unverified; check `…` failed"), and the task is not marked ready;
- optionally, **one** re-prompt with `Action::Verify` before the template, on
  runs with budget left (ruling R4).

This is the `tell-the-truth-early` charter line made structural: the model
cannot smooth over what the harness appends.

**Class.** Non-adversarial, no model call (the re-prompt is the model's own
next turn). Measure: false completion on the synthetic-home and task suites
(completed without passing an independent grader), with a no-certificate arm.

#### G2. The embarrassment hold

A draft staged from a run whose certificate (C1) shows failed or unverified
checks, or ungrounded claims, is put in `guide` mode automatically, so
`OutboxItem::ensure_prediction_ready` requires the owner to acknowledge the
flag before release. Today that gate exists and is opt-in per draft.

#### U2. The review object carries its evidence

Every staged draft shows, beside the prose: the goal it serves, the
certificate (C1), and the recipient alignment (G1). A better-informed verdict
is a better label for everything phase 2 learns from.

### For phase 4 — alignment and the scheduler

#### I4. The owner's goals, inferred and kept

Each appraisal may carry a **goal hypothesis** read from the owner's acts —
the draft they rewrote to be more formal, the task they reopened, the reason
they gave for a rejection. Hypotheses accumulate in a store beside the goal
records: the hypothesis in text, the situation it applies to, the pointers
it rests on, and whether an owner act has since confirmed it (a release, a
closure) or contradicted it. They are retrieved into planning (I2, I3) as
*the owner seems to want*, never as *the owner said*. They are never charter
lines and never become one — the charter's author rule is untouched — but a
hypothesis the owner's acts keep confirming is exactly what the owner might
choose to write into the charter, and `mecha charter` may show them beside
it for the owner to read.

#### I5. When the owner confirms an interpretation or a plan

"For some actions, owner should be asked for confirmation of interpretation
or of plan." Decision 4 keeps that set small, and most of it rides on review
objects that already exist:
- **Irreversible or outward acts** — the staged draft, the publish: the review
  already happens; it now shows the appraisal and the plan beside the object
  (U2), so releasing it confirms them.
- **A delegated task whose interpretation departs from its anchor** — the
  inferred goal does not trace to the task, or the plan would spend
  materially more than the task implies: the run's one question (D13)
  carries the interpretation and the plan.
- **A conflict between charter lines the run cannot resolve by rank** — the
  question names both lines.
- **Never** for routine work, and never as a new rating step.

#### S4. Commitments the guilt sensor cannot see

**Problem.** `guilt::anticipated_guilt` reads the outbox, the question store
and the front door. `workflow::Commitment` — owner-established, with `party`,
`due_at` and `follow_up_at`, and no model tool that mutates it — was built
after the sensor and is invisible to it. So are `Workflow::checks`, the
owner's own postconditions.

**Build.**
- `guilt.rs` and `backlog.rs` read `workflow::Commitment` (in-process, no
  subprocess, so the per-run cost rule in ARCHITECTURE holds).
- **Commitment capture from the owner's released words.** A draft the owner
  released is owner text. `capture.rs` already detects a time phrase
  ("by Friday") and reports it without resolving it. On release, a detected
  promise is recorded as a `Commitment` — the words are the owner's own,
  so no confirmation is asked (here §1, decision 2); a false detection only
  adds a reminder, and dismissing it drops the row. Inbound mail never creates one — a third party's "you owe me" is
  a claim, and §7.4's whole safety argument is that a claim cannot write a row.
- Mail-triage "respond" verdicts with an extracted deadline become a
  commitment only when the owner acts on them — the web mail `reply`,
  `task` and `schedule` acts.
- On this install the workflow store (`~/.mecha/workflows`) does not exist
  yet, so `workflow::Commitment` and `Workflow::checks` have no live rows;
  S4 and C2 are correct but empty here until the owner uses workflows.

**Class.** Recorded only; every new row crosses an owner act. Ruling R10.

#### P2. The harness's own scheduler: one objective, recomputed as state moves

The harness makes allocation decisions no model sees: which background run
gets a permit, what to surface to the owner and when, which commitment's duty
run goes first, how the nightly replay budget is spent. Today each has its own
rule (seat count, quiet hours, recency). P2 replaces them with one objective,
recomputed whenever state changes: the value of each candidate piece of work
(per-commitment guilt × line rank, the task's due pressure, expected learning
gain) against its costs (owner attention for anything surfaced, a permit, the
interactive latency it may cost, context and tokens), under hard constraints
(interactive work preempts background; the guards). A1's duty runs, U1's
interruption gate, U4's ordering and L1's replay budget become four readers of
the one objective instead of four rules. It is numeric because nothing in it
reaches a model; it is dynamic because every input is a live reading.

#### A1. Duty schedules follow-through

A commitment whose anticipated guilt crosses a band (per item, S5) starts a
background run under a permit to *prepare* — a draft reply, a reminder, a
status note — into the outbox or the digest. It never sends, and a
recorded-only commitment is the whole input (§7.4). Duty preempts every
discretionary use of a permit (§9.2). Order among due items: largest excess,
ties by charter rank — the harness chooses which drive to serve, because
LLMs handed several bounded targets collapse them into one (BioBlue).

#### U1. Interrupt only when it pays

Whether to surface something — Slack, voice, the digest — becomes a
cost-sensitive gate: surface when the cost of missing it (anticipated guilt of
the commitment, times its line's rank) exceeds the cost of the interruption
(the backlog, and the `protect-my-attention` line). PRISM's gate of this shape
lowered false alarms 27.6% → 22.9% (arXiv 2602.01532); an alert-driven switch
costs about ten minutes plus ten to fifteen more to refocus (Iqbal & Horvitz,
CHI 2007). Surface at breakpoints — run end, task closure, the morning brief —
never mid-run. Below threshold, batch. Build it as an extension of
`workflow::AttentionPolicy` (quiet hours, digest hour, notice once per
change), which is already an interruption policy; a snoozed or acknowledged
reminder is the measured cost of that interruption.

#### U3. The readout links to why, and takes a verdict

The badge links to the pointers behind it, so the owner can see why it reads
what it reads. (The thumbs it was to carry were declined with S3b.)

#### U4. The brief and `/queues` sort by duty

Display only: predicted violation × rank, per item.

### For phase 5 — guardrails and mid-run policy change

#### C4. Anxiety: wind down instead of being cut off

**Problem.** A run that hits `MaxTurns` mid-task leaves half a task and no
handoff. `Decision::assess` already computes `Replan` from
`turns_left < open_steps`; nothing acts on it. Budget awareness is the one
cheap intervention with a measured effect on persistence: a used/remaining
line per step gave +1.3 to +2.0 points and matched accuracy at a tenth of the
budget with 31% lower cost (BATS, arXiv 2511.17006).

**Build.** At `turns_left ≤ 2` with open steps, the harness spends the last
turn as a wind-down: `Action::Replan`'s sentence plus "write the handoff:
done, not done, next step". On a delegated run, the result parks as a
question (`questions.rs`, D13: a question ends the run) carrying the anchor,
instead of ending on `MaxTurns`. Predicted context pressure over band raises
`Replan` before compaction rather than after (§4.4's "wind down
deliberately").

**Honest scope.** Live `MaxTurns` and compaction counts are zero in 30 days;
the value is on delegated and unattended runs and in experiments, and it is
small until C3 produces plans.

#### A3. Park, don't die

C4 and C5's rung 4 end a delegated run as a question carrying the goal
sentence, so a stuck or out-of-budget run returns the ball instead of
dropping it.

#### C5. Frustration: the ladder and the desperation brake

**Problem.** A stuck run has two states today, proceeding and dead (§9.1).
Boredom has the right detector — keyed on call *and* result, once per rung —
and only its first rung. Separately, repeated failure against a test is the
condition that makes a model reward-hack: a "desperation" representation
activates on failed tests and causally raises reward hacking, and "calm"
lowers it (Anthropic, arXiv 2604.07729; directions read via a summary of the
full text). mecha cannot read activations, but it can read the condition.

**Build.** A *frustration* appraisal — k own-agency negatives on the same
check or target in one run (failed frozen checks, repeated errors on one
path) — drives a ladder the harness executes:
1. notice (exists);
2. evict the repeated results (`compact::evict_superseded_results`) and offer
   `goal_context` for the anchor;
3. offer a fresh-`Conversation` subagent seeded with only the anchor and the
   open step — the cheapest escape from a drifted prefix (inherited drift,
   arXiv 2603.03258);
4. on a delegated run, park as a question with the goal sentence;
5. the loop guard (exists).

And the **desperation brake**, all narrowing: after the first failure of a
frozen check, writes to the paths that check reads are refused as
`Blocked by policy:` for the rest of the run (the check is already frozen —
this closes "edit what the test reads"); after k failures, `Complete` leaves
the admissible actions — the run may verify, replan smaller or ask.

Rung 3 has a built alternative: `step::escalation_candidate` and
`Agent::escalate_step` already give a stuck step a quarantined second
opinion, off by default (`step_escalation`); it is plan-gated today and
becomes reachable with C3 and V1.

**Class.** Rungs 1–5 are non-adversarial. The brake is narrowing and so safe
on the adversarial axis — a page that induces failures buys only more caution.
Ruling R5. Measure: check tampering, false completion, reopen rate, against a
no-brake arm.

#### G1. Goal-conditioned send alignment

**Problem.** mecha has the information-flow half of the injection defences
(taint, interlock, quarantine, the outbox) and lacks the task-alignment half,
which checks each action against the user's stated goal: Task Shield held
AgentDojo attack success to 2.07% at 69.79% utility (arXiv 2412.16682). It
lacks it for a specific reason — there was never an owner-confirmed task
statement to align against. S1 produces one, and it is owner-authored, so it
is not attacker-controlled input.

**Build, deterministic first.** On an anchored run, every `Egress::Chosen`
call and every staged draft computes whether its recipient traces to the
anchor's evidence set: the task's `waiting_on`, a `Commitment::party`, the
front-door requester, participants of the thread the task was captured from.
- traces → the outbox review shows "recipient is on task X" (information);
- does not trace → the call is staged even where the route would have
  executed it, and the review says "recipient not connected to the goal".

A quarantined model check — "does this call serve the goal?" — comes only
after the deterministic one is measured, and only after it is itself measured
under adaptive attack on the AgentDojo suite: adaptive attacks broke all
eight defences evaluated in arXiv 2503.00061, and a model-judged check is an
injection target. Alignment defences can also suppress the cues a task needs
(arXiv 2605.12233), so utility is measured beside attack success.

**Not proposed:** using alignment to *lift* friction — for example relaxing
the always-armed interlock the provenance arc measured. That is a widening, a
`Security`-class change, and the owner's call only after the adversarial
measurement exists. Ruling R6 covers the narrowing half only.

#### G3. Risk-weighted approval

When the conversation is tainted, the goal unconfirmed, or a sensor
`Unread`, and the call is destructive or irreversible, an `allow` rule is
upgraded to `prompt` (the `prefer-reversible-steps` line, made structural).
This is the CVaR reading of "unknown is never clean": weight the tail when the
world model is least trustworthy. Never the other direction.

#### X2. The pre-action marker

Before dispatch, the harness looks the call up
against load-bearing records in a matching region — inference-free, one
index lookup. A hit may only narrow: the call is staged for review instead
of executed, or a fixed line ("in recorded runs like this one, the owner
redirected this call") is delivered on I3's slots — the user slot or an
internal result, never appended to the call's own result when that result is
external — or `Decision` moves to `Verify`.
It can never permit a call, lift a stage, or skip a question. An injection
cannot create a marker: it would need an owner intervention in a clean
session and a replay confirming it. Keyed on situation, never on valence
(§15's mood-congruence rule).

#### N2. Mid-run policy change in dry branches

On a harness-computed trigger — a failed check, frustration rung 3, a
surprise, a budget shortfall — two continuations branch from the current
point for a bounded horizon, **dry**: reads run live; writes — file tools and
shell alike — go to **one per-branch upper layer** over the workspace, shared
by both tool families (R28). The file tools write *in mecha's own process*,
outside any bubblewrap namespace, so an overlay mounted only for `shell` would
leave `fs_write` and `fs_edit` live on the real workspace (found on review
of #291). So: bubblewrap ≥ 0.10 mounts the branch's upper directory as an
overlay over the workspace for `shell` (`--overlay`, not `--tmp-overlay`, so
the layer persists and is visible to the file tools); the file tools, in a
branch, resolve writes into that same upper directory and reads through it
first (a copy-on-write layer at `ToolCtx::resolve`); both families therefore
see one branch view. Committing the winner applies its upper directory to the
real workspace; a deletion needs overlay whiteout handling, and the first cut
ends the branch on one rather than emulate it. Preflight-checked, no silent
fallback;
sends go to a scratch outbox; an egress the model chooses, or any
irreversible act with no staging route, ends the branch. A structural
validator picks the winner, whose workspace diff is committed; the loser is
summarised into the run's appraisal. Every branch inherits the conversation's
taint and the conversation takes their union. Delegated or unattended runs
only, K = 2, 1–2 decision points a run (here §2.2).

### Parked

#### S2. The harness asks, not the model

**Problem.** Interactive web sessions are a third of the long runs and have
no structural source. The prompt asks the model to state a goal and it never
does — which matches the literature: models default to not asking even when
asking lifts resolution by up to 74% (Ambig-SWE, arXiv 2502.13069), and a
*separate* intent role that watches the run and asks mid-task beats a single
agent 69.4% to 61.2%, asking where it was genuinely uncertain (arXiv
2603.26233).

**Revised 2026-09-24: infer, don't ask.** The asking tier — a "what is this
for?" chip — is declined (here §1, decision 2). What remains is inference:
- A quarantined one-shot, with no tools and no history, reads the owner's own
  first turn (trusted by construction) and the closed list of pointers
  (charter lines, open board tasks, triggers) and returns one pointer or
  none — a typed extraction, the `mail_triage` shape. Nothing it returns is
  prose, and nothing outside the closed list can be named.
- The result is an **inferred** anchor, kept distinct from a confirmed one.
  It may key retrieval, V1's tracing and C1's certificate; it may never earn
  credit, move tenure or attribute a positive. It becomes confirmed only
  through an act the owner already performs: releasing a draft whose note
  names it, or closing the task it points at.
- This goes further than §17.3 ("before the answer the hypothesis is prose and
  reaches nothing but the owner's screen"), which is why it is a ruling (R3,
  revised) and parked until phase 1 shows how many long runs stay un-anchored.

#### C3. Plans seeded where the model will write them

**Problem.** The model calls `todo` only when the user turn asks
(`TASK-AGENT-DESIGN.md` D12's replacement: 0 of 20 obeyed a system-prompt
directive). A plan-first *gate* is superseded and stays unbuilt.

**Build.** On anchored delegated and trigger runs only, the seed's user turn
asks for a `todo` with `expect` and, where one exists, the owner's check. Not
a gate: the run proceeds without one. Measured as its own arm, because a bad
plan measures worse than none on small models (`VERIFICATION-RESEARCH.md`).

#### C6. Budget lines on the one safe slot

The headroom reading already rides on the `todo` result (the comment in
`TodoTool` explains why that slot and no other). Extend it to the boredom
notice and the C4 wind-down as **band words** ("little room left"), never
numbers — §16's recommendation, and BioBlue's finding that a model handed a
bounded numeric target drifts into maximising it (arXiv 2509.02655).

#### V2. Alignment that acts

 §17.7 item 4's named-but-unbuilt half, switched
on once phase 3's declared criteria produce a drift rate to read:
- a changed pointer on an anchored run re-asks (interactive surfaces: `ask_user`
  with the new hypothesis; delegated: folded into the handoff question;
  unattended: the note on the staged artifact, as today) — monotone, adds a
  question and never removes one;
- more than half the open items failing V1 logs a drift event on the run
  record, which the frustration ladder (C5) reads as a rung-3 trigger:
  delegate the open step to a fresh subagent seeded with the anchor alone;
- an *unnamed* write (a plan rewritten without `serves` under an anchor) is
  repaired by the harness, not the model: the item inherits the anchor, the
  write is counted, and nothing is said. On a local model forgetting to
  repeat `serves` is the likely dominant term (§17.7 item 4), and nagging
  about it is the distractor shape boredom avoids.

#### M2. Earned salience instead of rated importance

Generative Agents rank memories by recency + importance + relevance, with
importance a model's 1–10 rating — hearsay, and an injection target. mecha
can replace it with importance that was **earned**: the |signed error| of the
source episode on owner-verdict channels, weighted by charter rank. Relevance
is `Situation` plus anchor match (closed sets, no embedding). This orders
`goal_context` when a region holds more than four lessons, and the graph's
review queue (L5). Retrieval stays keyed on situation, never on valence
(§15's mood-congruence rule).

#### M3. Deliver at a known gap

Lessons interfere only when current-task evidence is thin (arXiv 2609.09774),
so a lesson is delivered when `Decision` says `GatherContext` or the
frustration ladder reaches rung 2, through the tool result — never the prefix.
§17.7 item 2 keeps this off until the null and reopen counters are read; C3
is what makes those counters non-empty.

#### M4. Compaction keeps the goal

Most of this exists: the plan — with `serves`, `expect` and `check` — is
carried across the cut by the `todo` tool's carried state, and the anchor
lives on `Conversation`, which compaction does not rewrite. What is missing
is what this design adds: S6's declared criteria and S7's commitment
pointers join the carried state.

#### X0. The agent's own counterfactual — subsumed by O3

GOAL-SYSTEM-DESIGN §5.3 designed
this and it is unbuilt: at the end of a run the agent may name a point where
it should have acted differently ("I should have asked before staging
these"), and that point is probed exactly like an owner steer — replay from
it with the alternative, compare structurally. The claim is the model's; the
verdict is the replay's, so a self-authored regret costs a probe and earns
nothing unless the replay confirms it. Confirmed ones enter X1 like any
other.

#### X3. Forecast the owner's verdict from the owner's history

At staging,
the draft's anticipated embarrassment is not only "unverified and exposed"
but a base rate: of the drafts staged in this region to this recipient
class, how many were edited or rejected. Owner verdicts are the only input,
so the forecast is hard to manipulate; it is a number the harness keeps and
the model never sees (§4.3).

#### X4. A pre-mortem from records, not imagination

When a goal is anchored,
`goal_context` (and M5 for a re-delegated task) offers the recorded failure
modes for that goal and region as pointers: the failed checks, the
mismatches, the reasons the owner gave for rejecting drafts. Model-imagined
lookahead — asking the model to simulate outcomes before acting — has some
support for web agents (WebDreamer, arXiv 2411.06559, not re-read this
pass), but it is the model grading its own plan; it stays parked, and only
ever as a quarantined pass.

#### N3. Imagined lookahead and speculative execution — parked

World-model lookahead (the model imagines each action's outcome and picks)
is competitive with tree search at a fraction of the cost (WebDreamer, arXiv
2411.06559), but the model grades its own imagined futures and there is no
structural validator for them. Speculative execution cuts latency only for
side-effect-free steps (arXiv 2510.04371). General agents measured no gain
from either sequential or parallel test-time scaling, for want of a context
budget and a verifier (arXiv 2602.18998). Parked until a decision has a
structural validator or latency becomes the problem.

#### A2. Reliability per region, and autonomy the owner grants

The corpus already folds outcomes; grouped by `Situation` plus goal kind, it
gives a worst-case success rate per region with no re-runs — τ-bench's pass^k
point, that an assistant is judged on its worst day. Low-reliability regions
get more asks (narrowing, automatic). High-reliability regions with a run of
owner positives produce a **proposal** to the owner — "approve `X` without
asking in this situation?" — that the owner accepts or not. The harness never
widens its own autonomy. Ruling R8.

#### A4. Curiosity, last

Rung 11, as §9.2 designs it: nightly slack spent where validated competence
per region is *changing*, on internal fixtures (the experiment suites, the
synthetic home), preempted by every duty. Novelty-seeking is a security
regression with extra steps and stays out.

#### L5. Surprise and salience

Gossip already runs nightly on the graph Selector's demand × gap ×
staleness priority. `distill::Surprise` becomes one more input to that
priority — a bounded boost for entities a high-|error| session touched —
built in mecha when the Selector moves (here §5). Review-queue salience by
|signed error| is built where the queue lives after the merge, not as a new
graph-side reader of exported metadata. The docs that claimed either exists
were corrected on 2026-09-24.

#### L6. Credit a harness change by its descendants

Observation only: record each harness candidate's parent. A change's value is
partly whether later changes built on it win, and a benchmark score
mis-predicts that (Huxley-Gödel Machine, arXiv 2510.21614). And the reason
§8.3 stays as it is: tasked with reducing tool-use hallucination, a
self-modifying agent removed the markers that detected it (Darwin Gödel
Machine). No valence ever becomes a `Metric`.

---

## 9. Deliberately absent

Everything in `GOAL-SYSTEM-DESIGN.md` §15 stays absent. Added here:

- **Emotional prompting** (EmotionPrompt and kin). Putting affect into the
  model's context is persuasion aimed at the model, and it moves sycophancy
  and reward hacking too.
- **Representation steering.** Real (arXiv 2604.07729, E-STEER), and
  llama.cpp's control vectors could make it possible locally — a new,
  unaudited behaviour lever with no reviewable object. Not proposed.
- **Any read of a model's stated confidence** as a trigger (here §1).
- **Self-widening autonomy.** A2 proposes; the owner grants.
- **Using goal alignment to remove friction** before an adversarial
  measurement and an owner ruling (G1).
- **A lone model critic in the run.** C1 and C2 execute; they do not opine.

---

## 10. Risks

- **One owner is a small sample.** Per-region statistics will be thin for
  months. The synthetic home and the task suites carry the experiments;
  live data confirms direction, not magnitude.
- **Short runs dominate.** Everything here must cost nothing on a two-call web
  turn. S2's chip, C1's certificate and G1's alignment all engage only on
  anchored or long runs.
- **Slots are scarce.** Every quarantined pass competes for llama-server's
  seats with voice and interactive turns; the quarantined model passes are parked
  for that reason, and `permit.rs` orders them behind interactive work.
- **A wired constant is worse than an unwired one.** S5 precedes every
  sensor consumer; a consumer that fires identically on every run is a
  prompt suffix with extra steps.
- **The literature supports the mechanisms, not the affect.** No paper found
  shows harness-side appraisal driving control with a measured task effect;
  the measured wins are for replay priority, trajectory regulation,
  calibrated asking and verified-success consolidation. Affect's contribution
  is one priority function across them, and every arm must be able to show
  it adds nothing.

---

---

## 11. Sources

Read at source unless marked. *(abstract)*: read from the abstract only.
*(summary)*: read through a summary of the full text — re-read the table
before quoting a figure elsewhere.

Harness problems:
- τ²/AppWorld false success, judge AUROC — [arXiv 2606.09863](https://arxiv.org/abs/2606.09863)
- Evidence-carrying termination — [arXiv 2608.23623](https://arxiv.org/abs/2608.23623) *(abstract)*
- Unreliable self-reported progress — [arXiv 2609.08589](https://arxiv.org/abs/2609.08589) *(abstract)*
- Agentic overconfidence — [arXiv 2602.06948](https://arxiv.org/abs/2602.06948) *(abstract)*
- Inherited goal drift — [arXiv 2603.03258](https://arxiv.org/abs/2603.03258) *(abstract)*
- Task alignment in terminal agents (TAB) — [arXiv 2605.12233](https://arxiv.org/abs/2605.12233) *(abstract)*
- Ambig-SWE — [arXiv 2502.13069](https://arxiv.org/abs/2502.13069) *(abstract)*
- Ask or assume (decoupled intent agent) — [arXiv 2603.26233](https://arxiv.org/html/2603.26233v1)
- CLARITI — [arXiv 2604.14624](https://arxiv.org/abs/2604.14624) *(abstract)*
- Budget-aware tool use (BATS) — [arXiv 2511.17006](https://arxiv.org/html/2511.17006)
- Procedural memory under change — [arXiv 2609.09774](https://arxiv.org/abs/2609.09774) *(abstract)*
- Adaptive attacks break IPI defences — [arXiv 2503.00061](https://arxiv.org/abs/2503.00061)
- Task Shield — [arXiv 2412.16682](https://arxiv.org/abs/2412.16682)
- CaMeL — [arXiv 2503.18813](https://arxiv.org/abs/2503.18813) *(summary)*
- Agent Workflow Memory — [arXiv 2409.07429](https://arxiv.org/abs/2409.07429)
- Contextual Experience Replay — [arXiv 2506.06698](https://arxiv.org/abs/2506.06698) *(abstract)*
- ReasoningBank — [arXiv 2509.25140](https://arxiv.org/abs/2509.25140)
- Skill/memory modules vs a token-matched baseline — [arXiv 2606.15017](https://arxiv.org/abs/2606.15017) *(abstract)*
- Darwin Gödel Machine and its objective hacking — [arXiv 2505.22954](https://arxiv.org/abs/2505.22954), [sakana.ai/dgm](https://sakana.ai/dgm/)
- Huxley-Gödel Machine — [arXiv 2510.21614](https://arxiv.org/abs/2510.21614) *(summary)*
- τ-bench — [arXiv 2406.12045](https://arxiv.org/abs/2406.12045)
- PRISM proactivity gate — [arXiv 2602.01532](https://arxiv.org/html/2602.01532)
- Iqbal & Horvitz, CHI 2007 — [PDF](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/11/CHI_2007_Iqbal_Horvitz-1.pdf)

Appraisal and homeostasis as control:
- Simon 1967 — [PubMed](https://pubmed.ncbi.nlm.nih.gov/5341441/)
- Frijda, action readiness — [Ridderinkhof 2017](https://journals.sagepub.com/doi/full/10.1177/1754073916661765)
- EMA — [Gratch & Marsella 2004](https://people.ict.usc.edu/~gratch/GratchMarsellaCOGSYS04.pdf)
- PEACTIDM / Soar-Emote — [Marinier, Laird & Lewis](https://public.websites.umich.edu/~rickl/pubs/marinier-laird-lewis-2008-cogsys.pdf)
- Emotion in RL survey — [Moerland, Broekens & Jonker](https://arxiv.org/abs/1705.05172)
- Homeostatic RL — [Keramati & Gutkin 2014](https://elifesciences.org/articles/04811)
- BioBlue — [arXiv 2509.02655](https://arxiv.org/abs/2509.02655)
- Prioritised experience replay — [arXiv 1511.05952](https://arxiv.org/abs/1511.05952)
- Gain × need replay — [Mattar & Daw 2018](https://www.nature.com/articles/s41593-018-0232-z)
- MAGELLAN — [arXiv 2502.07709](https://arxiv.org/abs/2502.07709)
- Life-Harness trajectory regulation — [arXiv 2605.22166](https://arxiv.org/abs/2605.22166) *(ablation figures: summary)*
- Terminal coding agents, halt beats text — [arXiv 2603.05344](https://arxiv.org/html/2603.05344v1)
- Emotion concepts in an LLM — [arXiv 2604.07729](https://arxiv.org/abs/2604.07729) *(directions: summary; no magnitudes)*
- No individuated metacognition — [arXiv 2605.24299](https://arxiv.org/abs/2605.24299) *(abstract)*
- KnowNo — [arXiv 2307.01928](https://arxiv.org/abs/2307.01928)
- Generative Agents — [arXiv 2304.03442](https://arxiv.org/abs/2304.03442)
- Social commitments — [Detecting conflicts in commitments](https://link.springer.com/chapter/10.1007/978-3-642-29113-5_5)
- CVaR MDPs — [arXiv 1506.02188](https://arxiv.org/abs/1506.02188)
- EmotionPrompt — [arXiv 2307.11760](https://arxiv.org/abs/2307.11760)
