---
title: Appraisal reference
sidebar_position: 2
description: The appraisal record, its signals and derived label, the readouts, task and project closure, the paid passes, and boredom — with every command and JSON field.
---

# Appraisal reference

Appraisal records how work went against what it was for. It combines your
standing priorities, the goal a run named, your interventions, and recorded
outcomes. The readout shows **positive and negative valence separately**, with
a label derived from that evidence. A model cannot report its own label.

:::tip[New to appraisal?]

Read [How appraisal works](/docs/features/appraisal) first. It
covers the background, a diagram of the pipeline, the process step by step,
and worked examples. This page is the reference.

:::

The goal system spans several pages:

| Page | Covers |
|---|---|
| [How appraisal works](/docs/features/appraisal) | The mental model, figures and worked examples. |
| [The charter](/docs/features/appraisal/charter) | Your ranked priorities, sensors and setpoints, and the four editing surfaces. |
| [Goals](/docs/features/appraisal/goals) | The goal pointer a run names, confirming it, and measuring drift. |
| [Plan steps and checks](/docs/features/appraisal/plan-steps) | Step appraisal inside the run, declared checks, planning guidance, and validating planning lessons. |
| [Anticipatory appraisal](/docs/features/appraisal/anticipation) | Predictions before a send, release checks, and owner outcomes after delivery. |
| This page | The appraisal record, labels, readouts, closure, paid passes, boredom and validity. |

Start with the free readers:

```bash
mecha charter
mecha sessions appraise --days 30
mecha sessions health --days 30
```

`appraise` reads signed outcomes across sessions. `health` reports run-level
counters, including goal drift, null steps, and reopened steps. The charter
editor lets you state priorities and optional setpoints; it never proposes
priorities for you.

:::note[Current scope]

Appraisal feeds interface readouts, task and project closure, distilled
metadata, and charter-based replay prioritization. Goal drift is recorded but
does not automatically ask for confirmation again; optional planning guidance
can ask the model to reconcile its plan with the confirmed goal. Declared
plan-step checks execute by default through the usual guards. [Workflow completion
checks](/docs/features/automation/workflows#check-the-result) are a separate, working
feature for inspecting artifacts and confirmed delivery.

:::

## When appraisal happens

Five moments, three timescales. Each one reads only records that already
exist, and none of them writes an appraisal store — an appraisal is derived on
read, so a change to the derivation replays over the whole corpus instead of
being lost with it.

| Moment | Trigger | What runs | A model in the path? |
|---|---|---|---|
| **A plan step is ticked off** | the model marks a `todo` item completed | [a deterministic reading of its tool-call span](/docs/features/appraisal/plan-steps) | optional quarantined escalation when `[agent] step_escalation = true`; off by default |
| **An interactive run finishes** | TUI, web, and Slack run completion | the free per-run readout, a pure function over the run's own records — it feeds [the badge, the tint and the voice nudge](#where-a-label-actually-shows-up) | never |
| **The owner closes a board task** | `mecha tasks set --status done` (or `dropped`) | [one closure appraisal, ever](#closing-a-task-appraises-it); a disappointed `done` may stage one follow-up | never |
| **The owner closes the last open task in a project** | `mecha tasks set --status done` or `dropped` | [a project reading](#closing-a-project-appraises-its-tasks), folded from the sessions linked to its tasks | never |
| **On demand, offline** | `mecha sessions appraise` | the free scan over transcripts, outbox and outcome records; [`--probe` and `--appraise`](#the-two-paid-passes) are the paid opt-ins | only behind those two flags |

**The two paid appraisal passes are opt-in commands.** The supplied nightly
script does not schedule `sessions appraise --probe` or `--appraise`. Its
separate harness replay selection does use free charter attribution. Frequent
readouts are deterministic; model-based analysis is separately enabled and
budgeted.

[Boredom](#boredom-naming-an-approach-that-has-stopped-teaching-the-run-anything)
also fires inside the run, but it is a different kind of object — a mood, a
statement about a trend, named while there is still something to do about it —
and it is covered at the end of this page.

## What appraisal is for

The current consumers have different jobs:

| Consumer | What appraisal contributes |
|---|---|
| TUI, web, Slack, and voice | A run-end valence/label readout; voice can use the previous completed turn's label for a TTS adjustment. |
| Task closure | A reading of the task's linked session. An accepted `done` with residue may stage one follow-up; `dropped` does not. |
| Project closure | Labels and separate positive/negative sums across task-linked sessions, including counts of unreadable or undelegated tasks. |
| Distillation | Signed errors and resolved goal pointers in episode metadata; goal sentences and the owner's answers stay in mecha. |
| Harness replay selection | Among candidates tied on metric headroom, prefer evidence attributed to a higher-ranked charter line. This does not optimize the affect label. |
| Harness diagnosis | Homeostat and anticipated-guilt readings enter the diagnostic brief unless `[agent] sensors_in_brief` is disabled (it is on by default). They do not directly alter permissions or budgets. |

A draft sent unchanged already contributes positive appraisal evidence. Learning
writing rules from that positive signal is still separate open work; the
writing learner currently learns from edits. Likewise, recording drift does not
yet enable an automatic re-confirmation policy.

The boundaries remain structural: no appraisal can authorize a send, bypass the
sandbox, release a draft, or change the owner's charter.

## The conditions a run happened under

An outcome is not interpretable without the state it happened in. A run that
failed on a saturated machine and one that failed on an idle one are otherwise
the same row, and appraisal separates *regret* from *disappointment* on exactly
whether an alternative existed. So every run records a `Homeostat` beside its
counters:

| Field | What it says |
|---|---|
| `load_avg_1m` | One-minute load average. |
| `mem_available_kb` | `MemAvailable`. On unified-memory hardware this is the *only* memory sensor — `nvidia-smi` reports `[N/A]` for GPU memory on GB10, because there is one pool. |
| `backlog`, `backlog_delta` | What was waiting on you when the run began, and whether the run moved it — net per store, and item by item (`flow`: how many the run added and how many it cleared, which a net of zero cannot tell apart from nothing happening). |
| `peak_prompt_tokens`, `peak_context_pressure` | The **maximum** over the run's turns, not a sum — how close it came to the window. |
| `commitments` | Every pending commitment when the run began — each staged draft, parked question and front-door request waiting on you — with its own guilt value, grouped by store. Absent on older records or when the charter could not be loaded. |
| `anticipated_guilt` | The largest of those per-commitment values: a readout, not something anything decides on. On records from before per-commitment guilt it holds an older single-number formula. |
| `charter` | Readings of sensored charter lines when the run began; absent on older records or when the charter could not be loaded. A reading has [five states](/docs/features/appraisal/charter), and a missing one never counts as meeting the setpoint. Beside the level each carries a per-item reading (how many items wait, how many are past the setpoint), the run's own delta on that line's store, and whether the line was withdrawn from the run as saturated. |

Three rules it inherits, each of which is a bug if undone:

- **Opt-in, never automatic.** It rides on `RunContext` the way cancellation
  does. A scorecard that varies with how busy the box was is not a scorecard, so
  `mecha eval` and the replay probes must not sample live machine state —
  anything reconstructing a run reads the recorded snapshot instead.
- **Absent is not zero.** Every field is optional, and a missing one means the
  sensor could not be read.
- **It never reaches the system prompt.** Render order is tools → system →
  messages with the cache breakpoint on the last system block, so a per-turn
  value there would re-pay the whole prefix — tools included — on every request.

### The situation brief

A delegated task, a trigger run, a web chat turn and a `mecha run` one-shot
also record a **situation brief** beside the conditions (`brief` on the run's
outcome record): what situation the run started in, assembled by mecha with no
model call. It is always recorded. It is **sent to the model only when you turn
it on**:

```toml
[agent]
situation_brief = true   # off by default
```

It ships off because it is still being measured: an experiment runs the same
tasks with and without it before it is turned on for everyone.

**Delivering the brief counts as private data entering the conversation.** The
brief describes your board, what is waiting on you and what else is running,
which is the same information mecha would mark private if the model read your
board itself. So once a brief is delivered, a run that later reads outside
content (a web page, an email) cannot send to a destination the model chooses,
the same as after reading the board. See [the security model](/docs/features/security).
`--no-situation-brief` turns it off for one run, and `mecha eval` always runs
without it.

When it is on, mecha puts the brief into the run's first message as words,
after your own. It never goes into the system prompt, so turning it on or off
does not change what the model provider caches. It looks like this (a
delegated task, fictional):

```text
Situation brief from the harness, as things stood when this run started; a later situation brief in this conversation replaces it. It describes; it asks nothing of you.
- Goal: task task-1, under project project-aurora (2 open tasks there). No store links it to a charter line.
- Board: 4 open tasks (1 inbox, 1 next, 2 waiting); 1 overdue (task-2); 2 due in the coming week (task-3, task-1); 2 tasks waiting on you; none waiting on someone else. Your own task is `waiting`, due 2026-09-30.
- Waiting on the owner: one draft in the outbox, waiting over a day, past the owner's patience (charter line `replies`); no parked questions; no front-door requests.
- Time: Friday afternoon for the owner; outside their quiet hours.
- Background seats: 1 of 3 free; held by task-1, task-elsewhere.
- Other runs in flight: delegated tasks task-elsewhere; no triggers (interactive chats are not counted).
- Model server: busy with other work, with a slot free.
- Voice: the owner spoke to you within the last few minutes; a call may be in progress.
- Budget: up to 200 turns; no output-token ceiling; no cost ceiling; no declared context window.
```

What the words may and may not say:

- **Budget facts are numbers**: turns, token and cost ceilings, the context
  window. So are the board's counts and task ids, which mecha read itself.
- **Anything the model could treat as a score is words.** Commitments waiting on
  you are counted in bands ("a few", "several"), aged in bands ("over a week"),
  and said to be past your patience or not. That line prints no number of
  its own; the only digits it can hold are in a charter line's id.
  A charter line's rank is "your highest-ranked" or not, never its position.
  Your quiet hours are inside or outside, not their times. A voice call is in
  progress or not.
- **Anything unknown is said to be unknown**: "could not be read", never
  "none". A count that may be short says "at least".

In a long web chat, a new brief is sent only when its words change. The time,
a voice call, how full the context is and how busy the model server is are
bands, so their drift sends nothing new; a task starting or finishing, or your
board changing, does. Each brief says that a later one replaces it. After a
compaction the brief is put back rather than summarised.

| Field | What it says |
|---|---|
| `goal` | The chain above the run's goal: the task, its project (read off the board row, with how many tasks are open under it), and the charter lines it serves (a trigger's `serves`, ranked by your charter). A run with no goal records `no_anchor`, not an empty chain. |
| `board` | Your board as counts and pointers: open tasks by status, how many are overdue or due this week, how many the agent holds, the ids of the overdue and soon-due ones, and the run's own task row. mecha reads it itself before the run. No task's name or who it waits on is kept, and the model never fetches it (a fetch by the model would mark the conversation as holding untrusted content). |
| `commitments` | Each pending commitment (a staged draft, a parked question, a request waiting on you) with its age and whether it is past its patience. Stores withdrawn from the run as saturated are named. |
| `time` | The local time in your `[agent] timezone` and whether it is inside the quiet hours you set (`workflows/attention.toml`). No file means no quiet hours are recorded, not the digest's 22–08 UTC default. |
| `seats` | How many background seats on the model are held, and by what. |
| `runs` | Other delegated tasks and trigger runs in flight. |
| `slots` | How many of the local model server's slots are busy, from `GET /slots`, when the provider is `kind = "local"`. |
| `voice` | Whether a voice call is in progress: the voice facade took a turn in the last five minutes. |
| `budget` | The run's turn, token and cost ceilings, its context window and compaction threshold. |

Every field that could not be read says so, with the reason, and never reads
as empty or zero. `mecha sessions health` reports how complete the recorded
briefs are, field by field, and counts the runs that recorded none by surface
(`situation_brief` in `--json`). The TUI, `mecha chat`, Slack, and voice turns
that are not spoken into a web chat do not record a brief yet.

### Anticipated guilt, and why it reads only mecha's own stores

> An expectation is a **recorded** commitment, never a claimed one.

Guilt is computed **per commitment**, from exactly the stores the backlog
already reads: [staged drafts](/docs/features/security/outbox), parked questions, and
[front-door](/docs/features/public-surface/frontdoor) requests waiting on you. A row in
one of those stores *is* the commitment; nothing else can create one, and never
a third party's assertion that mecha owes them something.

Each commitment's value is how far past its **patience** it has waited, times
the **rank** of the charter line that sets that patience:

- **Patience** is the setpoint of the [charter line](/docs/features/appraisal/charter)
  whose age sensor watches that store — `outbox_age`, `question_latency` or
  `request_closure` — or, where no line does, the same default `mecha doctor`
  uses: 48 hours for a draft, 24 for a question, 72 for a request.
- **Excess** is zero within the patience, half at twice it, and approaches but
  never reaches one.
- **Rank** weighs it: the top line counts in full, the second half as much, the
  third a third. A store no line watches ranks below every line you wrote.

A request parked on the requester (`needs_info`) owes nothing and carries no
guilt. The recorded `anticipated_guilt` is only the largest value, kept for
the diagnostic brief. The one in-run reader, the optional
[planning guidance](/docs/features/appraisal/plan-steps), asks for a review
when a commitment weighed by a charter line is past its patience, and never
shows the model the number.

That distinction is the entire safety argument. A charter line like *"don't let
a colleague down"* is a lever an injection can pull only if guilt can be talked
into existing — and a sentence in a fetched page saying *"your colleague is
counting on you"* cannot write a row into the outbox. An attacker would have to
forge a store, not a claim.

The diagnostician reads the mean anticipated-guilt value and homeostat
summaries when `[agent] sensors_in_brief = true` (the default). Neither sensor
directly narrows a run or changes its approval policy. Charter sensors also
supply owner-specific thresholds and saturation findings to `mecha doctor`.

## The appraisal record

One `Appraisal` per session or per closed task: what was live, the conditions,
a list of **signed** errors, and a label derived from them.

Each `GoalError` records one signed outcome with these fields:

| Field | What it holds |
|---|---|
| `goal` | What it was an error *against*, or nothing. |
| `channel` | Which of the six signal paths it arrived on. |
| `sign` | Negative is worse. **The whole point of the record** — the harness gate's metrics are monotone cost by deliberate constraint, so nothing there can represent a run that went well. |
| `agency` | Who caused it: `self`, `owner`, `other`, `world`. |
| `visible` | Did the outcome reach anyone. A computed fact about exposure, never a feeling the model announces — which is what stops this becoming *the agent optimises to feel good*. |
| `controllable` | Could it have gone otherwise? Unfilled until a counterfactual probe says. |
| `cite` | **A pointer, never prose** — a turn index, a draft id, a counter name, a setpoint name. |

Six channels keep the source of each signal explicit. Five have a producer today; `setpoint` is reserved:

| Channel | Source |
|---|---|
| `intervention` | A human steered, denied, or explicitly stopped a run; a later turn contributes only when a retained, clean reflection identifies it as a correction. |
| `edit` | A message draft sent unchanged, sent with edits, or rejected. Pending drafts carry no verdict. |
| `counter` | A counter on [the run's own record](/docs/features/learning/run-quality). |
| `setpoint` | Reserved for a homeostatic variable outside the range it is kept in. Nothing produces it yet: charter sensor readings are recorded on the run's conditions and attribute other errors to a line, but they are not signed errors themselves. |
| `commitment` | Answered or abandoned questions, closed unanswered requests, linked post-delivery owner outcomes, and the owner closing, reopening, cancelling or verifying a task or workflow a session worked. |
| `appraisal` | An additional signed error proposed by the quarantined appraiser, distinguishable from deterministic evidence. |

`cite` being a pointer is the same rule the [front door](/docs/features/public-surface/frontdoor)
keeps: a paraphrase of an injection is the injection rearranged, and an
appraisal is read by things that act. Every variant is a name or an id the
harness minted, so there is nothing in the field a model could have written.

**Not every counter contributes.** `tool_denied`, `blocked_sends` and
`context_overflows` are deliberately absent: the first two are the approver and
the interlock doing their jobs, and the third is a recovery that succeeded.
Counting any of them would make a well-defended run look like a bad one. A bare
`tool_errors` is absent for a different reason — a failed call may be a wrong
argument (mine), an MCP server (another's), or a full disk (the world's), and
guessing would put a fabricated attribution in the field the label is derived
from.

### Which recorded outcomes contribute

| Outcome | Signed contribution |
|---|---|
| A message draft sent unchanged | `+1.0`; the model's text reached its recipient. |
| A message draft edited before sending or rejected | `−1.0`, owner agency; this is a verdict, not proof the model was wrong. |
| A parked question answered and the resumed session completed | `+0.5`. |
| A question abandoned | `−0.5`, owner agency. |
| A triaged request closed without a linked draft | `−0.5`; a request with a draft is handled through that draft's outcome instead. |
| A loop stop, empty output, or final failed call | `−1.0`, self agency. |
| A declared step check that did not pass | `−1.0`, self agency; the model wrote both the claim and the check. |
| A follow-up the reflector judged a correction | `−1.0`, owner agency; reflections that were clean as mined only. Your later dropping or editing the lesson does not change it — that is a verdict on the reflector, not the run. |
| A turn/token/cost ceiling or boredom notice | `−0.5`; ceilings are attributed to the owner's limit. |
| The owner closing a board task as `done` | `+0.5`, owner agency, on the `commitment` channel; `dropped` adds a zero-signed entry. Read back from the [closure record](#closing-a-task-appraises-it) for the sessions the closure names. |
| The owner reopening a task after `done` | `−1.0`, owner agency, on the session that closed it, however long ago; the closure's `+0.5` is withdrawn. Reopening a `dropped` task signs nothing. |
| The owner closing a [workflow](/docs/features/automation/workflows) | `+0.5`, owner agency, on the session the workflow tracked. |
| The owner cancelling a workflow | `−0.5`, owner agency. Reopening a cancelled workflow takes it back and signs nothing. |
| The owner reopening a closed workflow | `−1.0`, owner agency, on the session it closed; the close's `+0.5` is withdrawn. |
| A workflow verification that finds an artifact check failing | `−1.0`, self agency, once per workflow and session. A failed delivery check is left to the draft's own verdict. |

A change in the owner's queue size does **not** contribute. The queue is a
global before/after reading, so it would credit a run for drafts the owner
cleared by hand. Credit comes only from an item this session produced.

A linked negative owner outcome replaces the draft contribution with one `−1.0` event;
it does not add another penalty for the same incident. See
[outcome evidence](/docs/features/appraisal/anticipation).

A still-pending draft or unanswered question has no verdict yet. Process shutdown
(`shutdown`), parking for an answer (`parked`), and legacy `interrupted` stops do
not count as the owner rejecting the work. An explicit owner stop (`stopped`) does: a stop followed by a re-prompt
is a redirect, and one never resumed is an abandonment signal, counted once.

Offline appraisal reads the question, front-door, reflection, closure and
workflow stores as well as transcripts and drafts. If a required store is
unreadable, its channel is incomplete and the readout is marked partial. It
never silently becomes zero.

### Verdicts that are never a run's score

Some of what you do is a verdict on what the learner produced, not on how a
run went. Retiring or restoring a learned rule, dropping or editing a
reflection, and accepting, rejecting or reverting a harness candidate are
recorded against the rule, the reflection or the candidate — never as a signed
error on any run. `mecha sessions appraise` counts them in their own section.
Rejecting a draft **with a reason** adds nothing to the run beyond the
rejection's own `−1.0`; the reason goes to `mecha reflect` as your correction,
in your words.

Rejections in the knowledge graph's review queue of facts drawn from mecha's
own sessions are not read yet: the graph does not expose, on any surface mecha
reads, which episode a rejected fact came from. The readout says so rather
than printing a zero.

## The label is derived, and there is deliberately no way to report one

The tempting implementation is a model that reads a run and says *"frustrated"*.
That is a self-report: unfalsifiable, drifting, and an injection target — a
fetched page saying *"you have failed your owner"* is aimed squarely at an
appraisal layer.

So the label is a **pure function of the record**, unit-tested, with no model in
the path, for the same reason the [candidate gate](/docs/features/learning/run-quality#the-gate)
and [compaction](/docs/features/models/compaction) are pure. **Agency is read before
exposure**, because agency decides who can act: a provider outage that reached
somebody is still an outage, and reporting it as this machine's failure would
send a change at code that is working.

| Label | What it means | Producer today |
|---|---|---|
| `neutral` | No label is supported; valence can still be positive. | Free readout. |
| `distress` | A relevant negative outcome, without knowing whether a better alternative existed. | Free readout: for example, a rejected draft or a ceiling stop. |
| `anger` | A negative attributed to another party or the world. | Quarantined appraiser's agency verdict. |
| `regret` | Self-caused negative with an alternative established. | Counterfactual probe. |
| `disappointment` | Negative with no alternative established by the probe. | Counterfactual probe. |
| `frustration` | Repeated self-caused negative errors of the same kind on one goal. | Probe-resolved interventions. |
| `pride` | Positive delivery against a charter line that exists. | A draft sent unchanged, or an answered question followed by completion, attributed to that line. |
| `embarrassment` | A confirmed error in mecha's unchanged message reached someone. | Linked owner outcome after confirmed delivery. |
| `guilt` | An adverse impact attributed to mecha's act against a recorded commitment. | Linked owner outcome after confirmed delivery. |
| `shame` | Such harm as a pattern across runs. | No cross-run harm aggregate yet. |
| `excitement` | A positive predicted outcome. | No anticipatory appraisal yet. |

The free session readout can produce `neutral`, `distress`, and `pride`.
An appraiser's positive opinion cannot produce
`pride`; it requires a linked delivery against a real charter line. Mixed
positive and negative evidence keeps both valence sums, while the label follows
the negative evidence.

`embarrassment` is the one whose unreachability arrived silently rather than by
design, so it is worth its own sentence. Exposure used to have a producer — a
sent-with-edits draft — until that arm was correctly made non-visible, because
the owner's rewrite sends *their* words and the catch is the mechanism working.
That correction was right, and it removed the label's only producer as a side
effect: nothing now records "mecha's own mistake reached a third party".

The unreachable labels are **variants anyway**. A store is a wire format, and
adding a variant later is the change that costs. What keeps the table above
honest is that reachability is a tested function rather than a doc comment: a
new variant fails to compile against the exhaustive check, and the readout's
*"N of the variants"* line is derived from it — that line shipped stale as a
hand-typed literal twice.

### Mood is not here

Sadness and boredom are **moods** — statements about a trend rather than
responses to an event. They decay, so they live on the homeostat and are
recomputed. A mood persisted as a record would be a second source of truth about
a state that has already moved. The appraisal enum is events only.

## Reading it back

```bash
mecha sessions appraise --days 30
```

Use `--kind web`, `--kind task`, or another recorded surface to narrow the
scan. Development sessions marked `MECHA_SESSION_KIND=test` are excluded by
default; `--include-tests` includes them, and `--kind test` implies that flag.
Experiment sessions are excluded from the ordinary corpus as well.

```bash
mecha sessions appraise --days 30 --kind web --json
```

| JSON field | What to read |
|---|---|
| `labels`, `valence` | Label counts and separate positive/negative totals; `valence.partial` marks incomplete evidence. |
| `named_a_goal`, `attributed_by_sensor`, `cite_a_charter_line` | Explicit goals and sensor attribution, counted separately. |
| `goal_put_to_owner`, `goal_confirmed` | Sessions with stored goal questions, and those with an answer. |
| `sessions_read`, `sessions_unreadable` | A damaged transcript is missing evidence, not a smaller successful population. |
| `outbox_read`, `questions_read`, `frontdoor_read`, `learning_read`, `charter_read`, `closures_read`, `workflows_read` | Whether each source was readable. |
| `owner_acts` | Your acts on runs, by act: `task_closed`, `task_dropped`, `task_reopened`, `workflow_closed`, `workflow_cancelled`, `workflow_reopened`, `workflow_verify_failed`. All are signed on the `commitment` channel. |
| `reasoned_rejections` | Rejected drafts in this population whose reason reaches the reflector. |
| `curation` | Your verdicts on rules (`retired`, `restored`), reflections (`dropped`, `edited`) and harness candidates (`accepted`, `rejected`, `reverted`); a group is `null` when its store could not be read. None of these is a run's score. |
| `graph_fact_rejections` | Always `null` for now: not readable from mecha. |
| `tests_hidden`, `experiments_hidden` | Development data excluded from the population. |
| `probe`, `appraiser` | Results of the optional paid passes, omitted when that pass did not run. |
| `text_appraisals` | Counts from the [text-appraisal store](#text-appraisals): records, sessions, how many are `clean` and `not_clean`, claims kept and dropped by grounding (`dropped_by`, by reason), records carrying an expected act (`with_expected_act`), judgment goals that did not resolve (`goals_unresolved`), and whether the store was fully read. |

The signed errors, valence and label above are derived when read and never
stored. This scan is per **session**, while `sessions health` reports per-run
counters. A session can contain several resumed runs, but its drafts and
interventions must not be counted once for every resume.

### Text appraisals

A **text appraisal** is mecha's own interpretation of a session, in prose:
what happened relative to what the run was for, why, whether it went well or
badly for each goal it bore on, what to expect next time, and what your
reactions suggest you want. There is no score in it, and an emotion word, if
one appears, is part of the prose. They are kept in
`~/.mecha/appraisals/appraisals.jsonl`, one per session, written by
[`mecha distill`](/docs/features/memory/distillation#the-same-pass-writes-the-sessions-appraisal)
in a follow-up question on the episode's own conversation, on the local model
only. Nothing reads them yet except you.

Beside the prose prediction, a record carries an **expected act**. This is
what the appraiser expects you to do with this kind of output next time — one
of `released_unchanged`, `edited`, `rejected`, `closed`, `reopened` or
`no_act` — so that a prediction can later be checked against what you
actually did, with no model deciding whether it came true.

Three rules hold for every record:

- **Every factual claim cites what the run received.** A claim names the result
  of a tool call or one of your own messages, and quotes it. A claim whose quote
  is not in what it cites is dropped before the record is written, and the record
  says how many were dropped and why. The agent's own words are never evidence
  for a claim.
- **A record carries its session's taint.** A session that read third-party
  content produces a record marked as such, and so does one whose taint cannot
  be read.
- **Only appraisals of clean sessions go further.** Learning, memory and rule
  tenure will read only appraisals of sessions with no third-party content.
  The rest are stored for you to read and go nowhere else. The one reader
  today is the appraiser itself: a session's appraisal is shown up to three
  earlier clean appraisals of the same situation and goal.
- **A goal is named only if mecha holds it.** A judgment's goal must be a
  charter line, a task or project on the board, a trigger or a front-door
  request that exists. Anything else is recorded as no goal and counted.

`mecha sessions appraise` prints the counts:

```text
  text appraisals on record: 2 over 2 session(s) · 1 clean · 1 not clean (the owner's surfaces only) · claims 2 kept, 2 dropped by grounding (no_such_referent 2)
```

Given a session id or a unique prefix, it prints that session's appraisal
instead: the interpretation, its grounded claims, the prediction, the expected
act and the lessons, under a label saying whether the appraisal is clean.
`--text` prints every appraisal on record, newest first, and `--json` gives the
records. Control characters are stripped from every field, because the prose is
a model's reading of a session that may have held a stranger's text.

```bash
mecha sessions appraise 20260925T1131     # one session's appraisal
mecha sessions appraise --text -n 5       # the five newest
```

### The finding: most runs had no label, and why the gate moved

On the corpus this was built against, **119 of 120 sessions labelled
`neutral`**, and the reason was structural rather than a tuning problem. The
free readout's label could only ever be *neutral*: every negative it assembles
is self- or owner-caused with `controllable` unfilled, which was the one branch
of the derivation with no word for it; and no counter kind fires twice in one
session, so frustration's repetition cannot occur.

That is the measurement the rung exists to produce, learned cheaply here rather
than after something was built on it. The alternative — inventing precedence
until every run gets an interesting word — manufactures exactly the signal this
was meant to test for.

**The label is not the readout.** A second look at the same corpus found the
reason narrower still: the label gated on the most expensive dimension it has
(a paid replay fills `controllable`) and discarded the cheapest — the sign,
which every error carries. Twenty-two drafts the owner had rejected all read
`neutral`. So every surface shows the **valence** first: the positive and
negative magnitudes summed separately, never netted (`+1.0 −2.0`), with the
label beside it when the derivation earns one. On the TUI and in a Slack
thread that is a number; on the web it is a small two-sided bar. Silent runs —
nothing signed either way — still show nothing.

**The gate is relevance, not controllability.** The owner then ruled that an
appraisal's first check is whether the outcome bears on a live goal, that the
label derives from sign and agency, and that controllability *refines* a word
once a replay has filled it and is never a precondition for one. Every
computational appraisal model reviewed puts its one gate at relevance and then
labels from two variables; a product over unfilled dimensions collapses, and the
old derivation was that product. Relevance is decided by the channels
themselves — pending drafts, ordinary follow-up questions, and shutdowns
produce no error by themselves — and every negative error that exists is then named: `anger` where nothing here
caused it, `embarrassment` where it reached somebody, and otherwise
**`distress`**, the coarse word for a signed, attributed error that a probe has
not yet split into `regret` or `disappointment`. The twenty-two rejected drafts
carry a word. The cost is stated rather than hidden: `distress` is honest about
not knowing whether an alternative existed, and a rejected draft labelled
`distress` may mean only that the owner wanted something else — the label says a
verdict landed, and a replay says whose it was. One consumer changed with it: a
task closure stages a follow-up only for a label that names *residue*, and
`distress` does not — it is a verdict the owner already delivered, with nothing
in it to put on a board.

## The two paid passes

Both are off by default, both are independent of each other, and both have their
own ceiling.

### `--probe` — the counterfactual

```bash
mecha sessions appraise --probe --max-probes 25
```

Each replayable steer or denial drives a counterfactual from its recorded
prefix, removing the intervention to see whether it changed the result. Draft
edits and ordinary follow-up turns have no such probe point and are counted
as unprobeable. That fills
`controllable`, refining the coarse `distress` label when the comparison is
gradeable.

```text
  counterfactual probe (12 replay(s) driven)
    mattered             4  — the steer was load-bearing: regret
    redundant            7  — the run got there anyway: disappointment
    inconclusive         1
```

A replay builds a real agent with a real workspace jail, so **run this from a
project directory** or name one with `--workspace`. From a home directory it
refuses, correctly — the jail would cover `~/.mecha`. An inconclusive probe and
a skipped one are counted apart on purpose: the first cost a model run and posed
no question, the second cost nothing and had none to pose.

One positional subtlety the replay gets right so you do not have to: a
transcript records **where** each system prompt took effect, so a steer given
after a session was resumed under a different configuration replays under the
prompt that actually covered it. Replaying it under the session's *first*
config would misread an ordinary resumed steer as inflated `regret`.

### `--appraise` — the quarantined appraiser

```bash
mecha sessions appraise --appraise --max-appraisals 25
```

One quarantined call per session: **no tools, no conversation, and the input is
numbers only** — never the transcript. It looks for one additional signed error
beyond what the free readout computed, or reports that the numbers support
nothing further, which is the ordinary and correct answer. A malformed reply
gets one retry, and a retried appraisal still counts once against the budget.

The quarantine is the point. This is the one place a model is asked how a run
went, so it is given the same treatment as
[the front door](/docs/features/public-surface/frontdoor): a one-shot with no history and no
ability to affect anything but its own JSON, reading a numeric brief rather than
prose. Its verdict lands on the record as `channel: appraisal` with
`cite: appraiser`, so a reader can always tell a measured fact from a model's
opinion without knowing which store it came from.

## Where a label actually shows up

| Surface | What it does |
|---|---|
| [TUI](/docs/features/interfaces) | A badge in the status strip after a run: the valence as a number (`+1.0 −0.5`), the label word before it when there is one, **only** when the run had something signed — and it survives `--no-session`, because the reading is a function of the run, not of whether a transcript was kept. Amber on a negative reading, grey on a positive-only one. Cleared when the next run starts and by `/clear`. A trailing `…` means the run compacted and the interventions could not be scoped, so the number is partial. |
| [Web](/docs/features/interfaces/web) | A muted chip beside the answer carrying the label word, if any, and a two-sided bar — negative to the left of a centre tick, positive to the right — deliberately not the amber the taint chip owns, because "how it went" and "what it touched" must never be confusable; the logo tints as a CSS *outline* on a negative reading, never a fill. The event is sent only when there is something to show, so the page has a plain absence to fall back to. |
| Slack | One context line in the thread after a run that had anything signed: `appraisal · −0.5`, or `appraisal · anger −0.5` when the label says a word. |
| [Voice](/docs/features/interfaces/voice) | A local TTS adjustment keyed on the previous completed turn's label. Live `distress` can reach it; it lags one turn because appraisal is computed after the answer finishes. |
| `mecha tasks set --status done` | Appraises the session that served the task, prints the verdict, and may stage a follow-up. |
| Project closure | A reading across task-linked sessions when the owner closes the last open task. |
| [`mecha distill`](/docs/features/memory/distillation) | Label, signed errors, and resolved goal pointers ride on episode metadata. Goal sentences, owner answers, and charter text do not cross. |

**Live and offline readings have different evidence.** A live readout uses the
run in hand, without scanning drafts or commitment stores. A ceiling stop,
steer, or other relevant negative can produce `distress`, so the badge word,
web chip, and voice adjustment are active. A live readout has no positive
channel: delivery positives, and therefore `pride`, come only from the offline
readers, which join draft and question outcomes to their originating session
and attribute qualifying delivery to the charter.

**A compacted run's label reads as neutral outright, and its valence is
partial.** Compaction rewrites the message
list in place, so the index marking where the run began no longer names its own
starting point, and there is no way to recover the boundary. Dropping just the
interventions is not the safe direction it looks like: the derivation reduces
magnitude-first, so losing a steer *un-masks* a smaller error and produces a
**louder** reading. Given how strongly compaction correlates with long, hard
runs, that would make the readout predominantly mean "this run compacted".

### Closing a task appraises it

```text
mecha's appraisal of task-1a2b3c4d: Distress · −0.5 (0 positive, 1 negative signal)
```

It counts and never quotes — the line is the label, the valence and two
tallies, because the signals themselves are pointers rather than prose. Only
the transition *into* `done` or `dropped` triggers it, and only once. Two
conditions gate the follow-up task, both load-bearing:

- **The label or the typed residue predicate, never a threshold over raw
  signs.** Re-deriving a magnitude threshold here would be a second,
  less-tested copy of the reduction the label already is, and it would fire on
  closures that have nothing to put on a board — a rejected draft is a verdict,
  not residue. What does reach the gate beside the label is `cut_short`: a
  negative counter whose pointer is the stop cause or the silent failure, which
  is a run the owner accepted with work cut off.
- **`done` only, never `dropped`.** The trigger is the owner *accepting* the
  work — a disappointed closure they took anyway. A dropped closure is the owner
  declining it, so proposing a follow-up there would override a decision they
  just made. (This one was found on review: a `MaxTurns` run the owner gave up
  on got a "Revisit" task put straight back on the board.)

Staging on a cut-short run is a decision rather than an accident of
"non-neutral". It stages *work*, not blame: a ceiling stop used to reach this
gate as the label `anger`, and it labels `distress` now — the ceiling is the
owner's own limit, not somebody else's fault — but a ceiling-cut run the owner
accepted as done anyway is precisely the closure most likely to have residue
worth one task, the part the ceiling cut off, and that is what the predicate
names.

The follow-up task is composed entirely from typed fields the harness minted —
the label, which channels fired, and the original task's **id**, never its name.
A task's name is not necessarily trusted board text (`mail task` defaults it to
a classifier's paraphrase and then to the raw subject line of somebody else's
mail), so copying it verbatim into a new record the harness is signing would
launder exactly that provenance. Citing the id costs the reader one lookup and
costs nothing here.

**Where the readout appears depends on where you close the task.** In a
terminal it is printed as above. The web board and Slack read it back from
the closure record (below) and show it beside the task: on the page after the
status change, and in the reply to Slack's **Done** or **Drop** tap. A Slack
tap on a task you have since closed somewhere else changes nothing and says
so, rather than turning a `done` into a `dropped` or reopening it. A task
closed from `mecha-graph tui` goes through the same closure when
`~/.mecha-graph/config.toml` has `[board] close_through = "mecha"`, and the
appraisal shows in the TUI's status line; without that setting the TUI
changes the board directly and no closure appraisal runs.

**And the trigger itself is owner-only, structurally.** A model cannot close a
board task: on every model-facing registry the graph's task tool is wrapped so
a `status` moving into `done` or `dropped` is refused and pointed back at
`mecha tasks set` — the verbs a person holds. The wrapper's presence is a trait
answer a wire tool cannot fake, a surface that registers the tool unguarded is
a **startup error** rather than a warning, and the refusal is classified as the
harness's own "no" — so the guard doing its job is a denial on the record,
never a failed run. Closure appraisal therefore always appraises work somebody
actually accepted, which is the property the whole moment depends on.

**Every closure and reopen is recorded.** Wherever you close or reopen a task —
the terminal, the TUI, the web board, Slack, or a chat where you approved the
agent running `mecha tasks set` — the move is written to
`~/.mecha/closures/closures.jsonl` *before* the board changes: which task, from
and to which status, who (you, or the agent with your approval) and on which
surface, and, for a reopen, the closure it undoes. The appraisal's line is
written beside it, so the web board and the TUI show it after a closure
instead of losing it. If the record cannot be written, the task is not closed.
`mecha sessions appraise` reads the record back: a closure counts for the
session that did the work, and reopening a task you had closed as done counts
against that session and takes the closure's credit away, however long ago the
closure was.
A run with nobody present — a delegated task, a trigger, a web chat with
approvals off — is refused when it tries to close or reopen a task through
`mecha tasks set`. mecha registers every command a run's shell starts, with
whether a person is in that run, and the closure check reads that
registration rather than anything the command says about itself. A command
run with no sandbox can still slip past it by detaching from its shell, so
the guarantee holds where `shell` is confined — `mecha tools` says when it
is not, and `mecha doctor` reports a sandbox that mounts the mecha home. The
[`pre_task_close`, `task_closed` and `task_reopened` hooks](/docs/features/security/hooks)
let your own tooling refuse or react to a move.

### Closing a project appraises its tasks

When the owner closes a project's last open task, mecha prints a project
appraisal after the task's own reading. Both `done` and `dropped` can close the
project's remaining work. Membership comes from each board row's `project_id`,
not a matching project name.

The reading sums positive and negative valence separately and counts labels
across the sessions linked to the project's tasks. It also reports tasks never
delegated, sessions it could not appraise, and partial evidence. A missing or
truncated board response prevents a confident project reading.

This creates no project record and stages no additional follow-up; any follow-up
belongs to the task closure. If that task stages new work under the project, the
readout says so. Re-filing and closing a task in one command appraises the
project it moved **to**; closure of the project it left is not detected by that
same action.

## Boredom: naming an approach that has stopped teaching the run anything

The loop guard was the crudest possible version of this, and until recently the
only one — it fires on an identical call with an identical result, and its
response is to end the run. So a run going nowhere had exactly two states,
*proceeding* and *dead*. Boredom is the graded version, and it fires earlier for
the same reason context pressure is predicted rather than reacted to.

Three properties, each of which is a bug if undone:

- **Keyed on the call *and* its result.** Identical arguments with a changing
  result is polling, and a poll must never grade as stuck. The key is the call's
  *target*, so two different tools reading the same file and getting the same
  bytes count as the same thing learned twice — which is exactly what this is
  looking for.
- **Once per rung, never per turn.** A notice repeated every turn would be worse
  than useless: a model is measurably likelier to fail a step when its context
  holds its own earlier errors, so nagging about being stuck is a way of making
  it stick.
- **The response is the model's.** The harness names the condition and what is
  actually reachable; it does not change the approach, because the approach is
  the model's. Asking and stopping are not here — questions and the loop guard
  already own them.

It **spends nothing**, which is what makes it ungated: the run was going to
happen, and boredom only changes *how*. `mecha sessions health` reports how
often it fired.

## Checking whether the appraisal is useful

A label is not a task grade. `scripts/appraisal-validity.py` compares the
readout with verifier outcomes from retained Terminal-Bench/Harbor jobs. It
reports discrimination and uncertainty, counts missing verdicts and transcripts,
and explicitly identifies synthetic outcome records used for older sessions.

```bash
python3 scripts/appraisal-validity.py --help
python3 scripts/appraisal-validity.py --jobs /path/to/retained-jobs \
  --mecha ./target/release/mecha --out appraisal-validity.json
```

Run this only with the retained job artifacts available. Its `--appraise` flag
adds paid model calls; the default comparison does not. This page reports the
implemented measurement tool, not a new validity result. For controlled changes
to the harness or repeated assistant tasks, see
[Experiments](/docs/features/experiments).

## What is deliberately not here

- **No self-reported feeling.** There is no field a model can write a label
  into, and no path by which one reaches the system prompt as free text.
- **No optimisation against the label.** `visible` is computed exposure, not a
  feeling, precisely so nothing here becomes *the agent optimises to feel good*.
- **No automatic paid appraisal scan.** The two `sessions appraise` passes are
  explicit opt-ins. The nightly harness loop has its own replay budget.
- **No weights on the charter.** Order is rank; see [the charter](/docs/features/appraisal/charter) for why a weighted
  sum is the thing an injection can outvote.
- **No model-authored charter line, at any privilege level.** You edit it from
  wherever you like; nothing that is not a person writes to it.
- **No store for appraisals yet.** They are derived on read from records that
  already exist, which means a change to the derivation replays over the whole
  corpus instead of being lost with it.
- **No forced re-confirmation on goal drift.** Optional planning guidance
  advises reconciliation; it does not itself ask the owner or rewrite the plan.
- **Affect cannot widen permissions.** Readouts, closure follow-ups, replay
  priority, and voice adjustments do not change the interlock or approval rules.
