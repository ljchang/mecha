# Appraisal wiring — design

**Status: proposed 2026-09-24, nothing built.** The rulings the build waits on
are listed under *Rulings* (here §10). `GOAL-SYSTEM-DESIGN.md` is the design of the signals themselves and
`ARCHITECTURE.md`'s goal-system section is their invariants; this file does not
restate either. It designs one thing: **how the signals the appraisal system
already computes become inputs to decisions the harness makes**, and in what
order to build that so each step is measured before the next.

The question it answers, in the owner's words: the appraisal system has many
features, and the label is only a readout of internal state conditional on the
appraisals — so how does any of it improve planning, task completion,
guardrails, learning, memory and the rest of the harness?

Method. Five passes on 2026-09-24 against `main` at `b6cfa73c`: an inventory
of every appraisal-family signal and its readers; a map of every harness
decision point and the rulings that constrain it; a mining pass over the live
store (aggregates only, `MECHA_SESSION_KIND=test` on every readout); and two
external literature passes — open problems in agent harnesses, and appraisal /
homeostasis used as control rather than as a label. Figures are from those
passes; sources are in here §14, with anything not read at source marked.

**Section references.** A bare §N is `GOAL-SYSTEM-DESIGN.md`'s. This file's
own sections are cited as "here §N", and its proposals by id (S1, C1, G1, L1,
M1, A1, U1).

---

## 0. The finding in one paragraph

**The appraisal system is an output pipeline with no input and no consumer.**
It turns records into signed errors, a valence and a label, and the label goes
to a badge. Every mechanism that could change what the harness *does* — plan
feedback, step checks, drift, goal-keyed lessons, anticipatory guidance — keys
on a goal pointer or a plan the harness waits for the model to write, and the
served model writes neither: in 30 days of real use, 0 runs named a goal, 0
carried an anchor, and 4 of the 79 runs long enough to warrant a plan wrote
one. The two sensors that would drive behaviour read constants (a stale
outbox pins both at their ceiling), and the one priority consumer — replay —
uses the goal signal only to break ties. So the build is not "more
appraisal". It is three things in order: **supply** the pipeline from what the
harness already holds structurally; **map each appraisal to a closed set of
harness actions**, the way functional theories of emotion say an emotion is
a readiness for a class of action rather than a word; and **ship every
mapping as a lever that is measured before it is on**.

---

## 1. What is wired today, measured

Reader classes: **A** changes the model's in-run behaviour, **B** changes a
harness decision, **C** owner-facing display, **D** recorded and unread.

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

Four doc/code disagreements found on the way, fixed in phase 0 (here §11)
rather than in this file: `distill.rs` says `meta.affect` gives the graph's review
queue a salience order (mecha-graph never reads it); §10.1's "high-surprise
sessions seed gossip" happens only through a person; ARCHITECTURE's "two
sensors whose only reader is the diagnostician's brief" undercounts (the
charter reading also reaches `Decision` and the doctor); and `of_session`
still puts `backlog_delta`'s +0.5 into the positive valence sum that the same
section calls context, not credit.

---

## 2. The idea: an appraisal is an action tendency

Every functional theory of emotion reviewed says the same thing, and it is the
owner's framing. Simon (1967): a serial processor serving several goals needs
an *interrupt* mechanism "having the properties usually ascribed to emotion"
to decide which goal holds the processor. Frijda: an emotion *is* a change in
action readiness, with control precedence over what is running. Where
computational models closed the loop (EMA, Soar-Emote, emotion-driven RL), the
affect controlled four things: interrupts, goal priority, strategy selection,
and the learning signal. The word is downstream of all four.

So the unit this design adds is not a new signal. It is a mapping:

> **Each appraisal names a closed set of admissible harness actions. The
> harness computes the appraisal from records, picks from the set by
> arithmetic, and enforces the pick structurally where it can. The model
> receives at most a fixed-vocabulary sentence. The label is the name of the
> action family that was primed.**

`planning::Action` is already this shape in miniature (`ClarifyGoal`,
`Verify`, `Replan`, …) with `Action::guidance` as its only prompt surface.
This design generalises it.

| appraisal (harness-computed) | trigger | admissible actions | adversarial? |
|---|---|---|---|
| **Anxiety** — predicted self-shortfall | predicted next request over band; `turns_left < open_steps` | compact early · replan smaller · wind down with a handoff · delegate to a fresh subagent | no |
| **Frustration** — repeated own failure on one target | ≥ k own-agency negatives on one check or target in a run | escalate the ladder (§5.3) · freeze the check's targets · withhold `Complete` | yes → narrow only |
| **Boredom** — no progress | same call *and* result repeated | evict repeats · consult · delegate · park as a question | no |
| **Surprise** — prediction residual | check failed; `forecast_miss`; outcome ≠ `expect` | gather context · re-verify · (offline) replay and reflect first | partly |
| **Goal uncertainty** | long run, no anchor; plan `serves` left the anchor; sensor `Unread` | ask one goal sentence · retrieve | yes → may only add asks |
| **Anticipated guilt** — a recorded commitment approaching violation | per-item age vs the owner's setpoint; `due_at` | surface first · schedule follow-through · refuse a send that would "discharge" it | yes → narrow; recorded only |
| **Anticipated embarrassment** — about to expose unverified work | staging a message with failed/unverified checks or ungrounded claims | verify or ground before staging · hold release for acknowledgement | yes → narrow |
| **Regret** — own, replay-confirmed negative | probe verdict | reflect first · spend validation budget here | no |
| **Pride / relief** — owner-verified positive | sent unchanged · answered · `done` with checks passed · a thumbs-up | consolidate a success example · credit rule tenure · *propose* a skill | no — and **never permits anything, never shown to the model** |
| **Curiosity** — flat competence in a region, slack available | nightly, no debt, permit free | spend replay/probe budget there, on internal fixtures | yes if it touches the web → fixtures only |

The last column is `GOAL-SYSTEM-DESIGN.md` §7's test applied row by row. The
"never shown to the model" on pride is newer than §7: steering a model toward
positive-valence representations measurably raises sycophancy, and toward
"desperation" raises reward hacking (Anthropic, arXiv 2604.07729). Affect stays
harness-side in both directions.

---

## 3. Principles this adds to the existing invariants

The existing ones — dispositions are monotone (§7.3), affect is a priority and
never an objective (§8.3), select prioritised and confirm uniform (§8.1),
nothing per-turn in the prefix (§4.3), an expectation is a recorded commitment
(§7.4), no mood-congruent retrieval (§15) — all hold and are not restated.
Five more, each with the finding that forces it:

1. **Supply before demand.** A consumer may key only on something the harness
   holds structurally — a task id, a trigger, a store row, an owner click —
   never on the model having complied with an instruction. *Measured: 0% and
   5% compliance with the two planning instructions this model was given.*
2. **Structural before textual.** Where an action is non-adversarial or
   narrowing, the harness performs it (evicts, parks, holds, freezes) rather
   than asking the model to. *A model can ignore injected text; it cannot
   bypass an execution halt (arXiv 2603.05344); removing Life-Harness's
   graded trajectory-regulation layer cost 3–87% relative accuracy across
   seven environments (arXiv 2605.22166, figures read via a summary of the
   full text).*
3. **Never gate on self-report.** No wiring reads a model's stated confidence,
   progress or completion. *Twenty frontier models show no individuated
   verbal metacognition (arXiv 2605.24299); agents succeeding 22% of the time
   predict 77% (arXiv 2602.06948).*
4. **A level sensor is read per item or as a change, never as a level.** A
   reading that sits over its setpoint on every run is a constant, and a
   consumer of a constant is a fixed prompt suffix. *Measured: the outbox-age
   line reads excess 0.944–0.972 on 126 of 126 runs, pinned by eight drafts
   older than nine days.*
5. **Shadow, then measure, then arm.** Every wiring ships first writing the
   decision it *would* have made beside the run (the `Message::planning`
   pattern), then as a `harness::Lever` with a `mecha exp` arm against a
   no-wiring control at matched budget, and only then on by default. *The one
   efficacy pilot tied 36/36 against 36/36; skill and memory modules lose
   their gains against a token-matched baseline, on this model family too
   (arXiv 2606.15017).*

---

## 4. Supply: getting goals, verdicts and commitments into the pipeline

Everything in here §5–§9 depends on this section. It is the cheapest part and the
one with the largest measured gap.

### S1. Structural goal anchors

**Problem.** Every goal-keyed consumer reads an empty anchor. The harness
already holds the pointer on the runs that matter and throws it away:
`commands::tasks::work_prompt` writes `Id: task-…` into prose and never sets
`Conversation::goal_anchor`; triggers have no goal field at all.

**Build.**
- `tasks work` sets the anchor to `GoalRef::Task(id)` before the run, with the
  task's project as the parent — the same seeding `questions::seed_anchor`
  does on a resume. A board row is owner-created (`kg_task_create`), so this
  is an owner-authored goal, not a model's.
- `Trigger` gains an optional `serves = "charter:<id>"`, validated against the
  loaded charter at trigger load and refused if the line does not exist. The
  trigger store is owner-only and has no configurable path, so it inherits
  the charter's author rule.
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

### S2. The harness asks, not the model

**Problem.** Interactive web sessions are a third of the long runs and have
no structural source. The prompt asks the model to state a goal and it never
does — which matches the literature: models default to not asking even when
asking lifts resolution by up to 74% (Ambig-SWE, arXiv 2502.13069), and a
*separate* intent role that watches the run and asks mid-task beats a single
agent 69.4% to 61.2%, asking where it was genuinely uncertain (arXiv
2603.26233).

**Build, in two tiers.**
- *No model.* At a structural moment in an un-anchored interactive run — the
  first outbox staging, or the fourth tool call — the harness shows a chip on
  the web and TUI: "What is this for?" with the charter lines and the open
  board tasks as a **closed pick list**, plus "none". One tap sets the anchor.
  Nothing is inferred; the owner picks from pointers that already exist.
- *Quarantined pass, later (phase 4, here §11).* A one-shot with no tools and no
  history reads the owner's own first turn (trusted by construction) and the
  closed pointer list and returns one pointer or none — a typed extraction,
  the `mail_triage` shape. The result pre-selects the chip; it never becomes
  the anchor without the tap (§17.3: "before the answer the hypothesis is prose
  and reaches nothing but the owner's screen").

**Class.** May only add a question (§17.3's monotone rule). The chip is
dismissable and fires at most once per conversation; `protect-my-attention`
is the reason for both limits. Ruling R3.

### S3. A one-tap owner verdict

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

### S4. Commitments the guilt sensor cannot see

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
  promise is *proposed* as a `Commitment` in the digest for one-tap
  acceptance. Inbound mail never creates one — a third party's "you owe me" is
  a claim, and §7.4's whole safety argument is that a claim cannot write a row.
- Mail-triage "respond" verdicts with an extracted deadline become a
  commitment only when the owner acts on them ("I'll reply").

**Class.** Recorded only; every new row crosses an owner act. Ruling R10.

### S5. Sensor hygiene

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
- An owner-set expiry for drafts (ruling R7): past it, a pending draft moves to
  `expired`, which signs nothing (it is not a rejection) and stops holding the
  sensor at its ceiling. Silence stays "not a verdict".

---

## 5. Control inside a run: capability, accuracy, performance

### C1. Honest completion — the evidence-carrying final answer

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

### C2. Checks the harness writes

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

### C3. Plans seeded where the model will write them

**Problem.** The model calls `todo` only when the user turn asks
(`TASK-AGENT-DESIGN.md` D12's replacement: 0 of 20 obeyed a system-prompt
directive). A plan-first *gate* is superseded and stays unbuilt.

**Build.** On anchored delegated and trigger runs only, the seed's user turn
asks for a `todo` with `expect` and, where one exists, the owner's check. Not
a gate: the run proceeds without one. Measured as its own arm, because a bad
plan measures worse than none on small models (`VERIFICATION-RESEARCH.md`).

### C4. Anxiety: wind down instead of being cut off

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

### C5. Frustration: the ladder and the desperation brake

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

**Class.** Rungs 1–5 are non-adversarial. The brake is narrowing and so safe
on the adversarial axis — a page that induces failures buys only more caution.
Ruling R5. Measure: check tampering, false completion, reopen rate, against a
no-brake arm.

### C6. Budget lines on the one safe slot

The headroom reading already rides on the `todo` result (the comment in
`TodoTool` explains why that slot and no other). Extend it to the boredom
notice and the C4 wind-down as **band words** ("little room left"), never
numbers — §16's recommendation, and BioBlue's finding that a model handed a
bounded numeric target drifts into maximising it (arXiv 2509.02655).

---

## 6. Security: narrowing only

Nothing here goes in front of the interlock, the jail, the sandbox or outbox
routing (§15). Every item adds friction or information; none removes any.

### G1. Goal-conditioned send alignment

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

### G2. The embarrassment hold

A draft staged from a run whose certificate (C1) shows failed or unverified
checks, or ungrounded claims, is put in `guide` mode automatically, so
`OutboxItem::ensure_prediction_ready` requires the owner to acknowledge the
flag before release. Today that gate exists and is opt-in per draft.

### G3. Risk-weighted approval

When the conversation is tainted, the goal unconfirmed, or a sensor
`Unread`, and the call is destructive or irreversible, an `allow` rule is
upgraded to `prompt` (the `prefer-reversible-steps` line, made structural).
This is the CVaR reading of "unknown is never clean": weight the tail when the
world model is least trustworthy. Never the other direction.

### G4. Affect never reaches the model

Codify with a test: no provider-encoded request contains an `Affect` word,
a valence, or a sensor number. Today it holds by construction
(`Message::planning` is dropped by both encoders); the test is what keeps a
future "helpful" status line from breaking it.

---

## 7. Self-learning

### L1. Replay priority is gain × need

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

### L2. Learn from what went right

**Problem.** Stated in S3: the learning store is 100% corrections.

**Build.** An owner-verified positive (S3's thumbs-up, a draft sent
unchanged, a question answered, a task closed `done` with its checks passed)
becomes:
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

### L3. Goal-stamped reflections and per-line tenure

Stamp `goals` on each reflection at mining from `appraisal::attribute_events`
(the event-time attribution), so `goal_lessons` and `goal_context` stop
returning empty. Then build §17.2's per-charter-line aggregate, with a rule's
tenure measured on its line's owner-verdict channels only — S3 and the outbox
verdicts, never counters.

### L4. Mismatch from harness checks

C2's checks are the first steady source of `Trigger::Mismatch`. A failed
check is the false-success label that arXiv 2606.09863 found model judges
cannot produce: a detector trained on the harness's own ground truth.

### L5. Surprise and salience, built or un-claimed

Either build what `distill.rs` and §10.1 claim — `Surprise` auto-seeds a
bounded `gossip` pass over closed graph ids; the graph's review queue orders
by the episode's |signed error| — or correct the two docs. Phase 0 corrects
them; phase 3 builds.

### L6. Credit a harness change by its descendants

Observation only: record each harness candidate's parent. A change's value is
partly whether later changes built on it win, and a benchmark score
mis-predicts that (Huxley-Gödel Machine, arXiv 2510.21614). And the reason
§8.3 stays as it is: tasked with reducing tool-use hallucination, a
self-modifying agent removed the markers that detected it (Darwin Gödel
Machine). No valence ever becomes a `Metric`.

---

## 8. Memory and context

### M1. The goal joins `Situation`

§17.3's goal key, now that S1 makes it non-empty. It joins recording,
matching, replay and validation together, and an absent goal never widens a
rule's scope (APPRAISAL-RESEARCH §8.4).

### M2. Earned salience instead of rated importance

Generative Agents rank memories by recency + importance + relevance, with
importance a model's 1–10 rating — hearsay, and an injection target. mecha
can replace it with importance that was **earned**: the |signed error| of the
source episode on owner-verdict channels, weighted by charter rank. Relevance
is `Situation` plus anchor match (closed sets, no embedding). This orders
`goal_context` when a region holds more than four lessons, and the graph's
review queue (L5). Retrieval stays keyed on situation, never on valence
(§15's mood-congruence rule).

### M3. Deliver at a known gap

Lessons interfere only when current-task evidence is thin (arXiv 2609.09774),
so a lesson is delivered when `Decision` says `GatherContext` or the
frustration ladder reaches rung 2, through the tool result — never the prefix.
§17.7 item 2 keeps this off until the null and reopen counters are read; C3
is what makes those counters non-empty.

### M4. Compaction keeps the goal

The anchor, the open acceptance criteria, unverified steps and the commitment
pointers become mandatory survivors of the cut, the way taint and carried tool
state already are. A summary that drops the confirmed goal is the goal-drift
path.

### M5. A task remembers its previous attempts

`work_prompt` seeds a re-delegated task with nothing about earlier sessions on
the same task. Add pointers, not prose: the prior sessions' ids, their
outcomes (valence, failed checks, whether a draft was rejected), and, through
`goal_context`, the clean reflections stamped with that task. The second
attempt at a rejected task should start from why the first was rejected.

---

## 9. Independence and usability

### A1. Duty schedules follow-through

A commitment whose anticipated guilt crosses a band (per item, S5) starts a
background run under a permit to *prepare* — a draft reply, a reminder, a
status note — into the outbox or the digest. It never sends, and a
recorded-only commitment is the whole input (§7.4). Duty preempts every
discretionary use of a permit (§9.2). Order among due items: largest excess,
ties by charter rank — the harness chooses which drive to serve, because
LLMs handed several bounded targets collapse them into one (BioBlue).

### A2. Reliability per region, and autonomy the owner grants

The corpus already folds outcomes; grouped by `Situation` plus goal kind, it
gives a worst-case success rate per region with no re-runs — τ-bench's pass^k
point, that an assistant is judged on its worst day. Low-reliability regions
get more asks (narrowing, automatic). High-reliability regions with a run of
owner positives produce a **proposal** to the owner — "approve `X` without
asking in this situation?" — that the owner accepts or not. The harness never
widens its own autonomy. Ruling R8.

### A3. Park, don't die

C4 and C5's rung 4 end a delegated run as a question carrying the goal
sentence, so a stuck or out-of-budget run returns the ball instead of
dropping it.

### A4. Curiosity, last

Rung 11, as §9.2 designs it: nightly slack spent where validated competence
per region is *changing*, on internal fixtures (the experiment suites, the
synthetic home), preempted by every duty. Novelty-seeking is a security
regression with extra steps and stays out.

### U1. Interrupt only when it pays

Whether to surface something — Slack, voice, the digest — becomes a
cost-sensitive gate: surface when the cost of missing it (anticipated guilt of
the commitment, times its line's rank) exceeds the cost of the interruption
(the backlog, and the `protect-my-attention` line). PRISM's gate of this shape
lowered false alarms 27.6% → 22.9% (arXiv 2602.01532); an alert-driven switch
costs about ten minutes plus ten to fifteen more to refocus (Iqbal & Horvitz,
CHI 2007). Surface at breakpoints — run end, task closure, the morning brief —
never mid-run. Below threshold, batch.

### U2. The review object carries its evidence

Every staged draft shows, beside the prose: the goal it serves, the
certificate (C1), and the recipient alignment (G1). A better-informed verdict
is a better label for everything in here §7.

### U3. The readout links to why, and takes a verdict

The badge links to the pointers behind it and carries S3's thumbs. The label
stops being the end of the pipeline and becomes the place the owner feeds it.

### U4. The brief and `/queues` sort by duty

Display only: predicted violation × rank, per item.

---

## 10. Rulings the build waits on

Numbered so answers can cite them. None is a security widening.

| # | ruling | default proposed |
|---|---|---|
| R1 | Triggers may carry `serves = "charter:<id>"`, validated at load | yes |
| R2 | A one-tap owner verdict is a new owner-verdict channel, ±1.0, admissible for rule tenure (§17.2) | yes |
| R3 | The harness may show a closed-list "what is this for?" chip once per un-anchored long interactive run; a quarantined pass may pre-select it later | tier 1 yes, tier 2 after measurement |
| R4 | Honest completion: template only, or template plus one `Verify` re-prompt | template only first |
| R5 | Desperation brake: refuse writes to a frozen check's read set; withhold `Complete` after k failures | yes, `k = 2` |
| R6 | A recipient that does not trace to a confirmed goal is staged even where routing would execute | yes |
| R7 | Pending drafts expire after an owner-set age, as `expired` (signs nothing) | owner picks the age |
| R8 | The harness may *propose* per-region autonomy grants; only the owner grants | yes |
| R9 | The live line `be-the-best` ("always finding ways you could have completed a task even better") reads close to the unbounded self-improvement line §15 warns about. The owner's to keep or reword; flagged, not proposed | — |
| R10 | Promises detected in the owner's released drafts are proposed as commitments for one-tap acceptance | yes |

---

## 11. Build order

Each phase ends with a measurement that decides the next.

| phase | builds | the measurement that ends it |
|---|---|---|
| **0 — supply and honesty** | S1 (task, trigger, front door, web pointers), S5 per-item readings, L3 stamping, G4 test, the four doc corrections of here §1, shadow records for every here §5–§9 decision | anchored share of long runs; `sessions health` goal fields non-null; readings no longer constant |
| **1 — verdicts and truth** | S3 thumbs, C1 certificate (template), C2 harness checks, L1 gain × need, L2 success examples | false completion on task and synthetic-home suites vs no-certificate arm; verdicts per week; replay candidates accepted per night |
| **2 — control and narrowing** | C4 wind-down, C5 ladder + brake, G1 deterministic alignment, G2 hold, G3 risk-weighted approval, M4 compaction survivors, M5 prior attempts | per-arm: tampering, reopen, handoff quality; dojo suite attack success *and* utility vs control |
| **3 — memory and follow-through** | S4 commitments, M1 goal key, M2 salience, M3 gap delivery (after §17.7 item 2 reads), A1 duty runs, U1 gate, L5 built, U2–U4 | owner-side latency on sensored lines; interruptions per day; lesson application vs budget-matched control |
| **4 — the model-judged half** | S2 tier 2, G1 quarantined check under adaptive attack, A2 proposals, A4 curiosity, L6 lineage | each against its own arm; nothing here ships on a synthetic result alone |

Outcomes the whole programme is judged on, from APPRAISAL-RESEARCH §8.5 and
EXPERIMENT-DESIGN Part II: independently verified task success, per-charter-line
owner verdicts (sent-unchanged and rejection rates), false completion, asks per
run, rework, interlock friction and dojo attack success, latency and tokens —
all at matched budget, with variance. **Not** the label distribution, raw
valence, lower sensor values or rule count; those are the instrument, not the
outcome.

### What each proposal buys, by the owner's seven axes

| | capability | accuracy | performance | independence | security | self-learning | usability |
|---|---|---|---|---|---|---|---|
| S1 anchors | ● | | | ● | ● | ● | |
| S2 harness asks | ● | ● | | | | ● | ● |
| S3 one-tap verdict | | | | | | ● | ● |
| S4 commitments | | | | ● | | | ● |
| S5 sensor hygiene | | ● | | | | | ● |
| C1 honest completion | | ● | | | ● | ● | ● |
| C2 harness checks | ● | ● | | | | ● | |
| C3 seeded plans | ● | ● | | | | | |
| C4 wind-down | ● | | ● | ● | | | |
| C5 ladder + brake | ● | ● | ● | ● | ● | | |
| G1 send alignment | | | | | ● | | ● |
| G2 embarrassment hold | | ● | | | ● | | |
| G3 risk-weighted approval | | | | | ● | | |
| L1 gain × need | | | ● | | | ● | |
| L2 successes | ● | ● | | | | ● | |
| L3 goal-stamped tenure | | ● | | | | ● | |
| M2/M3 salience, gap delivery | ● | ● | ● | | | | |
| M4/M5 goal survives, prior attempts | ● | ● | | ● | | | |
| A1 duty runs | | | | ● | | | ● |
| A2 earned autonomy | | | | ● | | | ● |
| U1 interrupt gate | | | | | | | ● |

---

## 12. Deliberately absent

Everything in `GOAL-SYSTEM-DESIGN.md` §15 stays absent. Added here:

- **Emotional prompting** (EmotionPrompt and kin). Putting affect into the
  model's context is persuasion aimed at the model, and it moves sycophancy
  and reward hacking too.
- **Representation steering.** Real (arXiv 2604.07729, E-STEER), and
  llama.cpp's control vectors could make it possible locally — a new,
  unaudited behaviour lever with no reviewable object. Not proposed.
- **Any read of a model's stated confidence** as a trigger (principle 3).
- **Self-widening autonomy.** A2 proposes; the owner grants.
- **Using goal alignment to remove friction** before an adversarial
  measurement and an owner ruling (G1).
- **A lone model critic in the run.** C1 and C2 execute; they do not opine.

---

## 13. Risks

- **One owner is a small sample.** Per-region statistics will be thin for
  months. The synthetic home and the task suites carry the experiments;
  live data confirms direction, not magnitude.
- **Short runs dominate.** Everything here must cost nothing on a two-call web
  turn. S2's chip, C1's certificate and G1's alignment all engage only on
  anchored or long runs.
- **Slots are scarce.** Every quarantined pass competes for llama-server's
  seats with voice and interactive turns; the phase-4 model passes are last
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

## 14. Sources

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
