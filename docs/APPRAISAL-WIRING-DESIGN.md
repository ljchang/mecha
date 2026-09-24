# Appraisal wiring — design

**Status: proposed 2026-09-24, nothing built.** The rulings each phase
waits on are in here §6. The evidence behind every claim here — what exists, what
reads it, what has been measured — is
[`APPRAISAL-INVENTORY-RESEARCH.md`](APPRAISAL-INVENTORY-RESEARCH.md)
(cited as *inventory §N*). `GOAL-SYSTEM-DESIGN.md` designs the signals and
ARCHITECTURE's goal-system section holds their invariants; this file restates
neither.

It designs one thing: **how the signals the appraisal system already
computes become inputs to decisions the harness makes, in an order where each
step is measured before the next.**

A bare §N is `GOAL-SYSTEM-DESIGN.md`'s; this file's own sections are "here
§N"; proposals are cited by id (S1, L1, C1, …), and each id's detail is in
the catalogue, here §8.

---

## 0. The finding

The appraisal system turns records into signed errors, a valence and a label,
and the label goes to a badge. Four measured facts decide what to do about it
(inventory §1–§4):

1. **Nothing supplies it a goal.** In 30 days of real use no run named a
   goal, no run carried a confirmed anchor, and 4 of 79 long runs wrote a
   plan. Goal inference, drift tracking, step checks, goal-keyed lessons and
   planning advice are all built and all idle, because each waits for the
   model to write something it does not write.
2. **Injected guidance did not help.** It tied on easy tasks and lost on
   harder anchored ones — 20/24 without it, 18/24 with it, four regressions.
3. **Learning is starved while verdicts go unread.** The nightly loop runs
   every stage and has had nothing to learn from for a week, while the owner
   gives verdicts daily — reopening a task, rejecting a draft with a reason,
   closing a workflow, curating a rule — that nothing reads.
4. **The sensors that would drive behaviour are constants.** A stale outbox
   pins both the charter sensor and anticipated guilt at their ceiling on
   every run.

---

## 1. Three decisions that shape the plan

**1. Wire consumers in order of evidence, not of ambition.**

| order | kind of consumer | why this order |
|---|---|---|
| first | **recording** — get goals and verdicts into the store | changes no behaviour; every other consumer is empty without it |
| second | **offline** — learning, replay priority, ordering of what the owner sees | cannot make a run worse; measurable with stage levers |
| third | **structural in-run actions** — the harness appends, holds, parks, freezes | deterministic and mostly narrowing; the model cannot ignore them |
| last | **injected text** — advice sentences in tool results | measured locally to hurt as often as help |

**2. Supply before demand.** A consumer keys only on something the harness
holds structurally — a task id, a trigger, a store row, an owner's click —
never on the model having followed an instruction. Levels are read per item
or as a change, never as a level.

**3. Shadow, then measure, then arm.** Every wiring ships first writing the
decision it would have made, then as a lever with a `mecha exp` arm against a
no-wiring control at matched budget, then on by default. The outcomes are
verified task success and the owner's verdicts — never the label, the
valence, a sensor value or a rule count.

**4. No added work for the owner** (ruled 2026-09-24). The system infers
goals, reflects, anticipates and measures from what the owner already does;
it does not ask for ratings, confirmations or goal statements the owner
would not otherwise give. Confirmation comes from acts the owner already
performs — releasing a draft, closing a task, answering a question the run
genuinely needed. So no one-tap verdict (S3b declined) and no "what is this
for?" chip (S2's asking tier declined); the evidence is the acts in
inventory §4, read.

These sit on top of the invariants that already hold and are not restated:
dispositions only narrow (§7.3), affect is a priority and never an objective
(§8.3), prioritised selection is confirmed on a uniform holdout (§8.1),
nothing per-turn enters the prefix (§4.3), an expectation is a recorded
commitment (§7.4), and no wiring reads a model's stated confidence.

---

## 2. What each appraisal is for

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

An appraisal is not a word; it is a readiness for a class of actions (Simon's
interrupt, Frijda's action tendency), and the label names the class that was
primed. So each appraisal maps to a closed set of harness actions, computed
from records and picked by arithmetic. `planning::Action` is already this
shape in miniature.

| appraisal (harness-computed) | trigger | admissible actions | adversarial? | phase |
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

---

## 3. The plan: five phases

Each phase closes one loop end to end, so it can be judged on its own
outcome. Phases 2 and 3 can run in parallel once phase 1 lands; phases 4 and 5 follow.

### Phase 1 — Evidence in: make the store tell the truth

*No behaviour changes. Everything later is empty without it.*

| # | work | proposal |
|---|---|---|
| 1 | Seed the goal anchor from what the harness holds: the task id on `tasks work` (with its project), the trigger's own name on a trigger run, the request id on a front-door run | S1 |
| 2 | Task closure and reopening become one recorded lifecycle event, from every surface — CLI, TUI, web, Slack, chat, and the graph TUI — with hook events beside `pre_tool` / `post_tool` / `session_end` | S8, R15 |
| 3 | Read the verdicts already given: task reopened after `done`, outbox reject reasons, workflow close / cancel / reopen / verify, rule and reflection curation, harness accept / reject / revert, graph review verdicts on facts a session claimed | S3a |
| 4 | Sensor hygiene: per-item readings and a per-run delta instead of a level; a saturated reading withdrawn from consumers and reported once | S5 |
| 5 | One commitment record and guilt computed per item from it | S7 |
| 6 | Keep the counterfactual verdicts steer and validation probes already pay for | X1 |
| 7 | A test that no affect word, valence or sensor number reaches a provider request | G4 |

**Done when:** ≥ 60% of long real runs carry an anchor; verdicts per week are
counted by channel and the unread channels of inventory §4 appear; the
charter reading and guilt vary from run to run; a closure from any surface
shows up in `sessions appraise`.

#### Phase 1 as pull requests

Each is independently reviewable and lands behind its own tests; none
changes what a run does. Dependencies are the only ordering.

| PR | scope | proposals | depends on | acceptance |
|---|---|---|---|---|
| **1a** | **Goal anchors from structure.** `GoalRef` gains `trigger` and `request` kinds (lenient on read — a closed enum in an append-only store is a wire format). `tasks work` seeds the anchor to `task:<id>` with the project as parent; a trigger run anchors to `trigger:<name>`; a trigger's optional `serves` is validated against the loaded charter at load; the front-door drain seeds `request:<id>`. | S1, R1 | — | a delegated and a trigger run each record a non-null anchor; `sessions health` shows them; a `serves` naming a missing line refuses the trigger at load |
| **1b** | **Task closure as a recorded event, core half.** One closure function; the append-only closure record (transition, actor, surface, sessions, time, reason); `pre_task_close` (may deny, fails closed), `task_closed`, `task_reopened` hook events; the CLI, TUI and web board call it; the web shows the readout from the record; an unattended or delegated run cannot close. | S8, R15 | — | every direct surface writes one record per transition; a reopen is recorded and joined to its closure; a denying `pre_task_close` blocks; an unattended close is refused on every route |
| **1c** | **Closure from Slack and the graph TUI.** `TaskDone` / `TaskDrop` in Slack's closed `Action` enum, through 1b; the graph TUI's status change calls mecha's closure (a mecha-graph PR). | S8, here §5 | 1b | a Slack and a graph-TUI closure each produce a record with the right surface |
| **1d** | **Read the verdicts already given.** Reopen signs per R16 from 1b's record; the reject reason reaches the reflector as an owner correction; workflow close / cancel / reopen / verify sign per R16b–e; rule and reflection curation and harness accept / reject / revert are recorded against the rule, reflection or candidate (R16f–h); graph review rejections of facts from `agent:mecha` episodes are recorded for L7. | S3a, R16 | 1b | `sessions appraise` shows each new channel on a fixture; none of R16f–h moves a run's valence |
| **1e** | **Readings per item.** Charter and backlog readings carry per-item age and count beside the level, and each run's delta; a line saturated for `SATURATED_AFTER_RUNS` is withdrawn from in-run consumers and reported once by the doctor. | S5 | — | on the live store the per-item reading varies run to run while the level stays saturated |
| **1f** | **One commitment record, guilt per commitment.** `workflow::Commitment` absorbs `anticipation::Commitment`; drafts, parked questions and accepted front-door requests are commitments by construction; guilt per item = excess over patience × line rank; `anticipated_guilt` becomes a readout (the maximum), with old records still readable. | S7, R12 | 1e | each pending commitment has its own value; the homeostat readout matches the maximum; no consumer reads the scalar |
| **1g** | **Keep the counterfactual verdicts.** Steer-probe and validation verdicts write a counterfactual record keyed by situation, goal kind and call class, from clean sessions with a readable tool surface only. | X1 | — | a `--probe` run leaves records a second read returns; tainted sessions leave none |
| **1h** | **Affect never reaches the model.** A test over both provider encoders: no `Affect` word, valence or sensor number in any request. | G4 | — | the test fails when a status line carrying a valence is injected |

1a, 1b, 1e, 1g and 1h can proceed in parallel. The phase-1 readout — anchored
share of long runs, verdicts per week by channel, per-item reading variance —
is added to `sessions health` by the PR that first produces each number.

### Phase 2 — Learning out: the nightly loop learns from that evidence

*Offline consumers only. They cannot make a run worse.*

| # | work | proposal |
|---|---|---|
| 1 | Attribute a correction by what the run was given — data error, behaviour error or gap — and mine a behaviour lesson only from a behaviour error (port mecha-graph's D3 contract) | L7 |
| 2 | Learn from what went right: drafts sent unchanged as writing exemplars, verified successes as examples and as contrast for the reflector | L2 |
| 3 | The anchor as a second goal source for reflections; rule tenure per charter line on owner verdicts, decided by a Wilson lower bound (port the graph's ladder); dormancy for rules whose region stops recurring | L3 |
| 4 | Replay and reflection priority = gain × need: \|signed error\| on owner-verdict channels × how often the situation recurs, uniform holdout unchanged | L1 |

**Done when**, in a lifetime experiment against the appraisal-off preset
(stage levers): `learn` forms rules again on live-shaped data; the share of
decisive validations rises; harness candidates find paired episodes that
discriminate; verified task success does not fall.

### Phase 3 — Honest completion: the first in-run consumer, and it is structural

*The consumer the literature supports most: false completion is the dominant
agent failure, and judges cannot catch it (C1's problem statement).*

| # | work | proposal |
|---|---|---|
| 1 | Checks the harness writes: grounding over a staged draft's dates and names; the owner's workflow checks; mismatches from their failures | C2, L4 |
| 2 | Acceptance criteria the agent declares with its goal, from a closed set the harness executes — one-sided until the owner confirms them | S6 |
| 3 | The goal validator: every plan item traces to the anchor, deterministically | V1 |
| 4 | The completion certificate, appended by the harness; a draft from an uncertified run is held for acknowledgement; the review shows goal, certificate and alignment | C1, G2, U2 |
| 5 | A re-delegated task starts with pointers to its previous attempts and why they were rejected | M5 |

**Done when:** false completion on the task and synthetic-home suites falls
against a no-certificate arm with `WORK_FLOOR` holding; owner rework on
delegated tasks falls.

### Phase 4 — Follow-through: commitments drive attention and preparation

| # | work | proposal |
|---|---|---|
| 1 | Commitments from owner acts: mail `reply` / `task` / `schedule`; promises in released drafts recorded automatically (dismissing one drops it) | S4 |
| 2 | Duty runs that *prepare* follow-through for a commitment approaching its setpoint — never send | A1 |
| 3 | Surface only when missing it costs more than the interruption, at breakpoints, extending `workflow::AttentionPolicy`; order the brief and `/queues` by duty | U1, U4 |

**Done when:** owner-side latency on sensored lines falls; interruptions per
day do not rise; nothing surfaced is dismissed as noise more often than
before.

### Phase 5 — Guardrails and stuck runs: narrowing controls on anchored runs

| # | work | proposal |
|---|---|---|
| 1 | Wind down before the ceiling and park a delegated run as a question instead of dying | C4, A3 |
| 2 | The frustration ladder and the desperation brake | C5 |
| 3 | A send whose recipient does not trace to the confirmed goal is staged; destructive calls under taint or an unconfirmed goal are prompted | G1, G3 |
| 4 | Pre-action markers from the stored counterfactual verdicts, narrowing only | X2 |

**Done when:** on the AgentDojo suite, attack success falls and utility
holds; per arm, check tampering and reopen rates fall and handoffs are
usable.

---

## 4. Parked, and what would unpark each

| item | why parked | unpark when |
|---|---|---|
| `goal_guidance` and every injected-advice form (C6, M3 gap delivery) | measured to hurt as often as help | phase 3 has produced plans and criteria worth advising on, and a new arm is designed |
| S2 — the harness *infers* a goal for un-anchored runs, from a closed list | goes beyond §17.3's confirmation rule; a model pass | phase 1 shows how many long web runs stay un-anchored, and phase 2 shows goals change what is learned |
| R7 draft expiry | owner ruling: not until the system has stabilised | the owner says so |
| C3 seeded plans; V2 re-ask and drift event | plans can hurt small models; no drift rate yet | phase 3's criteria produce a rate to read |
| M1–M4 memory (goal key, earned salience, gap delivery, criteria across compaction) | nothing goal-linked to retrieve yet | phase 2 forms goal-linked rules |
| X0 self-authored steers; X3–X5 verdict forecasts and prediction scoring | need stored verdicts and recorded outcomes | X1 holds records and owners record outcomes |
| A2 earned autonomy; A4 curiosity; L5 surprise-seeded gossip; L6 lineage | lower value, or a new use of slack | phase 2 and phase 4 are measured |
| every quarantined model pass (S2 tier 2, V1 relevance, G1 model check) | an injection target, and a slot | the deterministic version is measured, and the model check survives adaptive attack |

---

## 5. mecha-graph: port on demand

The owner's direction (2026-09-24): mecha is the harness and mecha-graph a
tool that should eventually merge into it. So a graph mechanism moves into
mecha core **when a phase needs it**, using the graph's version as the
reference implementation, and no new cross-repo reader is built as the
long-term shape. Inventory §5 has the full overlap table.

| phase | what it ports |
|---|---|
| phase 1 | the board's closure path — the graph TUI closes through mecha's one closure event (S8) |
| phase 2 | the D3 correction contract; the ladder's Wilson-bound tenure; decay as rule dormancy; the Selector's demand term as L1's *need* |
| phase 3 | `verify.rs` folded into `grounding.rs` — one grounding primitive |
| phase 4 | review-on-use's verdict queue as the shape of `mecha review` |
| phase 5 | pack flags (contradicted / denied / stale) as anticipation evidence |

---

## 6. Rulings, by phase

Numbered as before so earlier answers still cite them. None is a security
widening.

| # | phase | ruling | default proposed |
|---|---|---|---|
| R14 | all | Mechanisms overlapping mecha-graph are built in mecha core, porting the graph's version; no new cross-repo readers | **stated by the owner, 2026-09-24** |
| R1 | phase 1 | A trigger run is anchored to the trigger itself (`trigger:<name>`); an owner-written `serves` link to a charter line is optional, never required | **ruled 2026-09-24: optional only** |
| R15 | phase 1 | Closing or reopening a task, on any surface, is one recorded event with hooks | **ruled 2026-09-24** (S8) |
| R16 | phase 1 | How the unread acts sign: a task reopened after `done`, at any age, −1.0 on the closing session, withdrawing its success; a rejected graph fact to L7's attribution only; R16a–R16h as tabled in S3 | **ruled 2026-09-24**, every item as proposed |
| R2 | — | A one-tap verdict channel | **declined 2026-09-24**: no added owner work (here §1, decision 4) |
| R7 | parked | Pending drafts expire after an owner-set age, as `expired` | **deferred 2026-09-24** until the system has stabilised |
| R12 | phase 1 | Guilt becomes per-commitment goal error toward another party; one commitment record; the homeostat scalar becomes a readout | **ruled 2026-09-24: per commitment** |
| R4 | phase 3 | Honest completion: template only, or template plus one `Verify` re-prompt | template only first |
| R11 | phase 3 | The agent may declare acceptance criteria from a closed set of harness-executed kinds; one-sided until the owner confirms them; never a charter sensor | yes |
| R10 | phase 4 | Promises detected in the owner's released drafts are recorded as commitments automatically — the words are the owner's own; a false detection only adds a reminder, and dismissing it drops it | yes |
| R5 | phase 5 | Desperation brake: refuse writes to a frozen check's read set; withhold `Complete` after k failures | yes, `k = 2` |
| R6 | phase 5 | A recipient that does not trace to a confirmed goal is staged even where routing would execute | yes |
| R13 | phase 5 | Stored counterfactual verdicts may narrow a matching call before dispatch | yes, narrowing only |
| R3 | parked | The harness may infer an anchor from the owner's first turn onto a closed list of pointers; inferred anchors key retrieval, tracing and the certificate, never credit or tenure; confirmation comes from acts the owner already performs | yes, when unparked (the asking chip was declined) |
| R8 | parked | The harness may *propose* per-region autonomy grants; only the owner grants | yes, when unparked |
| R9 | — | The live charter line `be-the-best` ("always finding ways you could have completed a task even better"). Unboundedness is not the issue — charter lines are attractors (here §2). §15's narrower worry is an unbounded line whose *object is the harness itself*, beside a loop that proposes harness changes; that pressure is held structurally, because no lane can accept a `Security`-class change. Flagged once; the owner's to keep or reword | — |

**Phase 1 has every ruling it needs** (R1, R12, R15 and R16 ruled 2026-09-24;
R2 declined; R7 deferred). The rest can wait
for their phase.

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

---

## 8. The proposal catalogue

The detail behind each id, grouped by the phase that builds it. Parked
proposals are at the end.

### For phase 1 — evidence in

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

**S3b — declined 2026-09-24 (here §1, decision 4).** A one-tap verdict
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

#### S8. Closing a task is a recorded lifecycle event, with hooks

**Ruled by the owner, 2026-09-24:** "closing a task via a chat, tui, slack,
or web should be recorded and have a hook just like pre/post turn and session
start/end."

**Today.** The closure appraisal lives inside the CLI verb `tasks set`. The
TUI's `/tasks` and the web board reach it (the web through that verb, with the
readout lost on the child's stderr); Slack has no close action; in chat the
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
  something a consumer reads (here §1, decision 2: a level is read per item,
  never as a level, and this one is the saturated level).
- **Every guilt consumer reads the same per-item value:** `Decision`'s
  `ReviewCommitment`, G2's embarrassment hold, A1's duty runs, U1's
  interruption gate, and the retrospective label.

What does not move: *an expectation is a recorded commitment, never a claimed
one* (§7.4) — the unification changes which code computes guilt, not what may
create a row. The wire formats are append-only, so the two old commitment
shapes and the scalar stay readable leniently. Ruling R12.

#### X1. Keep the verdicts

Every steer and validation probe writes a
counterfactual record: the `Situation` scope keys, the goal kind, the tool
and a closed-set call class (tool name and argument *shape*, never argument
values or prose), the verdict (load-bearing / not / inconclusive), and the
pointer to the intervention. Only clean-provenance sessions, by the learning
gate's own rule, and only probes whose recorded tool surface still exists
(`surface::Fidelity` — before it, 12 of 13 probes were inconclusive). This
is storage for work already paid for.

#### G4. Affect never reaches the model

Codify with a test: no provider-encoded request contains an `Affect` word,
a valence, or a sensor number. Today it holds by construction
(`Message::planning` is dropped by both encoders); the test is what keeps a
future "helpful" status line from breaking it.

### For phase 2 — learning out

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

### For phase 3 — honest completion

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

#### M5. A task remembers its previous attempts

`work_prompt` seeds a re-delegated task with nothing about earlier sessions on
the same task. Add pointers, not prose: the prior sessions' ids, their
outcomes (valence, failed checks, whether a draft was rejected), and, through
`goal_context`, the clean reflections stamped with that task. The second
attempt at a rejected task should start from why the first was rejected.

### For phase 4 — follow-through

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
  so no confirmation is asked (here §1, decision 4); a false detection only
  adds a reminder, and dismissing it drops the row. Inbound mail never creates one — a third party's "you owe me" is
  a claim, and §7.4's whole safety argument is that a claim cannot write a row.
- Mail-triage "respond" verdicts with an extracted deadline become a
  commitment only when the owner acts on them — the web mail `reply`,
  `task` and `schedule` acts.
- On this install the workflow store (`~/.mecha/workflows`) does not exist
  yet, so `workflow::Commitment` and `Workflow::checks` have no live rows;
  S4 and C2 are correct but empty here until the owner uses workflows.

**Class.** Recorded only; every new row crosses an owner act. Ruling R10.

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

### For phase 5 — guardrails and stuck runs

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
of executed, or the tool result carries a fixed line ("in recorded runs like
this one, the owner redirected this call"), or `Decision` moves to `Verify`.
It can never permit a call, lift a stage, or skip a question. An injection
cannot create a marker: it would need an owner intervention in a clean
session and a replay confirming it. Keyed on situation, never on valence
(§15's mood-congruence rule).

### Counterfactual anticipation (X), across phases

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
  verdict that cost a model run lives only in one readout.
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

X1 is built in phase 1 and X2 in phase 5 (above). The rest is parked:

#### X0. The agent's own counterfactual

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
for?" chip — is declined (here §1, decision 4). What remains is inference:
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

#### M1. The goal joins `Situation`

§17.3's goal key, now that S1 makes it non-empty. It joins recording,
matching, replay and validation together, and an absent goal never widens a
rule's scope (APPRAISAL-RESEARCH §8.4).

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
