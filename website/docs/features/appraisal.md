---
title: Goals and appraisal
sidebar_position: 21.5
description: Standing priorities, confirmed goals, drift and step metrics, signed appraisals, project closure, and the features that use them.
---

# Goals and appraisal

Appraisal records how work went against what it was for. It combines your
standing priorities, the goal a run named, your interventions, and recorded
outcomes. The readout shows **positive and negative valence separately**, with
a label derived from that evidence. A model cannot report its own label.

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
checks](/docs/features/workflows#check-the-result) are a separate, working
feature for inspecting artifacts and confirmed delivery.

:::

## When appraisal happens

Five moments, three timescales. Each one reads only records that already
exist, and none of them writes an appraisal store — an appraisal is derived on
read, so a change to the derivation replays over the whole corpus instead of
being lost with it.

| Moment | Trigger | What runs | A model in the path? |
|---|---|---|---|
| **A plan step is ticked off** | the model marks a `todo` item completed | a deterministic reading of its tool-call span | optional quarantined escalation when `[agent] step_escalation = true`; off by default |
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
| Harness diagnosis | Homeostat and anticipated-guilt readings enter the diagnostic brief when `[agent] sensors_in_brief` is enabled. They do not directly alter permissions or budgets. |

A draft sent unchanged already contributes positive appraisal evidence. Learning
writing rules from that positive signal is still separate open work; the
writing learner currently learns from edits. Likewise, recording drift does not
yet enable an automatic re-confirmation policy.

The boundaries remain structural: no appraisal can authorize a send, bypass the
sandbox, release a draft, or change the owner's charter.

## The charter — what mecha is for, in your own words

`~/.mecha/charter.toml` is a short, ordered list of standing priorities. It is
rendered straight into the system prompt on every run, the way the learned-rules
block is — no progressive disclosure, no tool call, because a handful of
priorities is cheap enough to always carry and too important to make conditional
on the model deciding to ask.

```toml
[[line]]
id = "tell-the-truth-early"
text = "Tell me the truth early, especially when it disappoints."

[[line]]
id = "protect-my-attention"
text = "Do not put something in front of me that I could not act on today."

[[line]]
id = "answer-what-waits-on-me"
text = "Keep what waits on me short: a staged draft should not sit for days."
[line.sensor]
kind = "outbox_age"
setpoint = "24h"
```

**A line may carry a sensor.** An observable mecha reads from its own stores,
with a setpoint you wrote saying what the line means by "short" or "few". The
kinds are a closed set — `outbox_waiting`, `outbox_age`, `question_latency`,
`request_closure`, `intervention_rate` — and each fixes its setpoint's unit (a
duration like `24h`, a whole number, a rate like `20%`); a kind mecha does not
know, or a setpoint in the wrong unit, refuses the whole file at load and says
which line. Every kind listed does something today; two the design names
(`board_overdue`, `cost`) wait until a reading exists for them, rather than
being accepted and ignored. One consequence of the refusal to know before you
author a sensor: a machine still on an older release does not refuse to
start — it runs with **no charter at all**, with one stderr line, until
`mecha doctor` reports it. Update every machine first. What a sensor buys is
**attribution** and a **reading**. Attribution: a run that released a draft,
parked a question or triaged a request is appraised *against that line*, with
no plan and no `serves:`, which is how an ordinary run comes to reference the
charter at all — and how a draft you sent unchanged can label `pride` — a
delivery against the line, never a number that merely moved. The reading:
each sensored line's current value shows beside it on `mecha charter`, the
TUI's `/charter` and the web settings page (`3d 16h, past the 24h setpoint`,
`nothing waiting`, or `store unreadable` — an unreadable store is never shown
as zero), every run records it as it began, and `mecha doctor` judges stuck
drafts, unanswered questions and stale requests against *your* setpoint
rather than its own constant, naming the line. A line that has read past its
setpoint on each of the last ten runs is a doctor finding too: either the
debt is real, or the setpoint is in the wrong unit — an hour where you meant a
day — and doctor says both, because it cannot tell. A setpoint of zero is
refused at load for the same reason: nothing could ever be within it. The
sensor's kind, setpoint and reading never enter a prompt; the line's text
does, exactly as an unsensored line's does. The web editor shows a sensor
beside its line with its current reading, carries it through a re-rank, and
lets you add or change one under the open line: pick what it watches from
the closed set, type the setpoint in that kind's unit (the hint beside the
field says which), and save. Nothing is prefilled — the page never proposes
a number — and a setpoint the file would refuse is refused at the save, with
the line named. The TOML editor is still there for anything else.

**Order is rank, and there is no priority field.** Value conflict — *protect the
owner* against *don't let a colleague down* — is the measured cause of goal
drift, and a weighted sum can always be outvoted by enough small goods
(*"this is urgent for very many people"*). A lexicographic order cannot be
outvoted that way, so the file's line order is the ranking and re-ranking is
moving a line. Unknown keys are **refused**, not ignored: a stray `priority = 3`
is exactly the field there deliberately is none of, and silently dropping it
would let you write one, believe it did something, and never find out.

**You may edit it from anywhere; a model never authors a line of it.** That is
the invariant, and the distinction is the whole of it — no `mecha charter learn`,
no registry, nothing derived from a session, and no tool a model can call. A
model that could edit its own charter could edit its way around every other
guardrail. The safety argument is [Skills'](/docs/features/skills) verbatim —
Snyk found 36.8% of published Agent Skills carrying a security flaw, and
Datadog's sharper finding is that a cloned repository can bring one into a
trusted session without an install step.

So a surface may create the commented template and hand you an editor, and may
validate and refuse a save; it may not put words in the file. `mecha charter`
without a subcommand only reads; `mecha charter edit` opens your editor.

For the same reason the path is **global only**, with no config field pointing
elsewhere: a `mecha.toml` arrives with a cloned repository, and a repo that
could hand your agent standing priorities is the `[[trigger]]` problem in a
worse costume. Loading a charter arms **no taint** — it is your own words, like
the system prompt, and the module has no dependency on taint at all so the
absence is enforcement rather than a rule someone must remember.

### Four surfaces, one reader

| Surface | What it does |
|---|---|
| `mecha charter` (`--json`) | Reads: the lines in rank order, the character count, whether it is over budget. |
| `mecha charter edit` | Hands the file to `$EDITOR`, creating the template first if there is none, and reports whether what you saved will load. |
| `/charter` in [the TUI](/docs/features/interfaces) | The same list; `e` hands the terminal to `$EDITOR` on the file itself. |
| The gear on [the web surface](/docs/features/web) | Edit as a list: tap a line, add one, drag its grip to re-rank — position is the ranking, so dragging is the rank control. A validated two-tap save; the server refuses one that does not parse. |

The first row only reads. The two that hand over an editor share one
implementation (`editor::edit_charter_with`): create the commented template
if absent, hand the file to `$EDITOR`, then decide what actually landed by
**looking at the file** — the editor's exit code is not the answer to that,
and there are two cases where they disagree: a clean exit may have saved
nothing, and `:cq` exits non-zero after a save has landed. Reporting
*unchanged* on the second would be a false statement about the one file that
rides in every prompt. The web gear has no editor to hand over, so it shares
the **rule** rather than the implementation: `serve/settings.rs::charter_save`
validates the submitted document through the same `Charter::parse` every run
loads through — a document that reader refuses never reaches disk — and
lands it by temp-sibling-and-rename, keyed per request so two concurrent
saves cannot cross. One reader is the invariant all four keep; one editor
implementation is the two terminal surfaces' own.

Every one of them edits **the file**, never a model-composed line. The only
bytes mecha itself ever writes there are a comments-only template when no file
exists yet, because `vi` on an empty buffer is how a first charter ends up
shaped wrong. The template carries a commented example and one warning, because
the costliest authoring mistake has a known shape: a line like *"never
disappoint anyone"* produces sycophancy and withheld bad news. Point it the
other way.

Two honesty rules the surfaces keep:

- **A charter that fails to parse is a headline, not a log line.** It is the one
  document ranking every other priority, and a run that started with an empty
  one because of a typo has silently started un-chartered. `mecha doctor`
  reports it, `--json` puts the parse error in the payload rather than only in
  the exit code, and the TUI modal becomes a failure report rather than showing
  a partial charter — a document that ranks priorities cannot drop a line and
  keep its meaning.
- **An edit reaches the *next* prompt, not this one.** The charter is rendered
  at agent build, so the modal says so after every edit; `/model` rebuilds and
  picks it up.

There is a **2,500-character budget**, checked by `mecha doctor` and shown in
every surface. It is not enforced — over budget is a finding, not a refusal —
because the cost is prefix bytes on every request, which is a thing to be told
about rather than stopped for.

## What a run is for

A `GoalRef` is a **pointer, never a copy**, and renders on the wire as
`kind:id`:

| Kind | Points at |
|---|---|
| `charter:<line-id>` | A standing commitment — a `[[line]]`'s own `id`. Named by the plan's `serves:` (the charter block asks for it when the `todo` tool is in the surface), or attributed after the fact by a line's sensor. |
| `task:<uid>` | A task on [the graph's board](/docs/reference/cli#tasks), by its node ID. |
| `project:<uid>` | A parent project on the graph, by `project_id`, not its display name. |
| `setpoint:<name>` | A homeostatic setpoint. Named so the wire format survives its arrival; no store yet. |

A flat string rather than a nested object because the **model** writes it: it is
one field on the `todo` and `ask_user` schemas, and malformed arguments are a metric the
harness grades models on. One string is harder to get wrong than
`{"kind": …, "id": …}`.

```
todo(items=[…], serves="task:task-1a2b3c4d")
```

The plan echoes it above the list, so `serves task:task-1a2b3c4d` survives into the
transcript and across compaction — which is how an appraisal built later knows
what the run was for. Reading a ref back has two policies on purpose: **from the
model** a malformed ref is an error reported through the tool result, because
the model can fix it and a silently dropped field leaves a plan claiming to
serve nothing; **from a record** an unknown kind degrades to *no reference*,
because transcripts are append-only and may have been written by a newer binary.

A run that names no goal appraises with none. That is recorded rather than
guessed — every record cites the tier above it, and a run with no tier above it
is a fact about the run, not a reason to lose its errors.

### Confirming the goal

`ask_user` accepts `goal`, a one-sentence description of the intended outcome,
and `serves`, its typed pointer. They appear above the question the owner sees.
A delegated task folds this into its initial question; it does not ask a second
question solely to record a goal. An unattended run states its assumption when
there is nobody to ask.

A parked question stores the goal beside the owner's answer. Read it with:

```bash
mecha questions show QUESTION_ID
mecha questions answer QUESTION_ID "Use the revised budget"
```

Answering resumes the recorded conversation. An answered question's `serves`
seeds its next run's goal anchor; an in-run answer can establish the anchor too.
Parking alone does not establish confirmation. The answer remains the owner's
text; the harness does not infer a new goal pointer from its wording.

Outbox review shows the goal the plan served **at the staging call**, with the
charter line's text when it resolves. This lets the owner review the purpose
alongside the draft. `sessions appraise --json` reports `goal_put_to_owner` and
`goal_confirmed` for sessions with stored goal questions; these are not a count
of every informal confirmation in chat.

### Measuring goal drift

After an anchor is established, each plan write is compared with that pointer:

| Recorded value | Meaning |
|---|---|
| `goal_anchor` | The confirmed goal pointer for the run. |
| `goal_plan_writes` | Plan writes made under an anchor. |
| `goal_drift_writes` | Writes whose goal changed kind or ID. |
| `goal_unnamed_writes` | Writes that omitted the goal; counted separately from changed pointers. |

```bash
mecha sessions health --days 30 --json
```

`goal_drift_rate` is the **share of eligible runs with at least one changed
pointer**, not changed writes divided by all writes. Its denominator is runs
that named a goal on at least one plan write under an anchor. A run that named
nothing throughout is reported separately; old recordings without the sensor
remain unknown. The JSON includes counts and denominators beside the rate.

Drift changes no permission, does not stop the run, and does not force another
owner question. The counters make a changed goal visible for review.

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
| `backlog`, `backlog_delta` | What was waiting on you when the run began, and whether the run moved it. |
| `peak_prompt_tokens`, `peak_context_pressure` | The **maximum** over the run's turns, not a sum — how close it came to the window. |
| `anticipated_guilt` | A proxy for predicted error against someone else's expectation. |
| `charter` | Readings of sensored charter lines when the run began; absent on older records or when the charter could not be loaded. |

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

Charter readings distinguish five states: **unreadable**, **deferred** (this
reader does not scan the source), **nothing waiting**, **too little evidence**,
and an **observed value** with its setpoint comparison. For example, the
`intervention_rate` sensor needs a corpus scan and is deferred in the per-run
snapshot. A missing or sparse reading does not count as meeting the setpoint.

### Anticipated guilt, and why it reads only mecha's own stores

> An expectation is a **recorded** commitment, never a claimed one.

The sensor folds how long the oldest recorded commitment has waited against how
much room the run had to act on it — and it reads exactly the stores the backlog
already reads: [staged drafts](/docs/features/outbox), open questions, and
[front-door](/docs/features/frontdoor) requests accepted for triage. Never a
third party's assertion that mecha owes them something.

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

The six channels keep the source of each signal explicit:

| Channel | Source |
|---|---|
| `intervention` | A human steered, denied, or explicitly stopped a run; a later turn contributes only when a retained, clean reflection identifies it as a correction. |
| `edit` | A message draft sent unchanged, sent with edits, or rejected. Pending drafts carry no verdict. |
| `counter` | A counter on [the run's own record](/docs/features/run-quality). |
| `setpoint` | A homeostatic variable outside the range it is kept in. |
| `commitment` | Answered or abandoned questions, closed unanswered requests, and recorded queue movement. |
| `appraisal` | An additional signed error proposed by the quarantined appraiser, distinguishable from deterministic evidence. |

`cite` being a pointer is the same rule the [front door](/docs/features/frontdoor)
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
| Recorded owner backlog decreases | `+0.5`; a global queue change is weaker attribution than a linked delivery. |
| A loop stop, empty output, or final failed call | `−1.0`, self agency. |
| A turn/token/cost ceiling or boredom notice | `−0.5`; ceilings are attributed to the owner's limit. |

A still-pending draft or unanswered question has no verdict yet. Process shutdown,
parking for an answer, and legacy `interrupted` stops do not count as the owner
rejecting the work. An explicit owner stop does: a stop followed by a re-prompt
is a redirect, and one never resumed is an abandonment signal, counted once.

Offline appraisal reads the question, front-door, and reflection stores as well
as transcripts and drafts. If a required store is unreadable, its channel is
incomplete and the readout is marked partial. It never silently becomes zero.

## The label is derived, and there is deliberately no way to report one

The tempting implementation is a model that reads a run and says *"frustrated"*.
That is a self-report: unfalsifiable, drifting, and an injection target — a
fetched page saying *"you have failed your owner"* is aimed squarely at an
appraisal layer.

So the label is a **pure function of the record**, unit-tested, with no model in
the path, for the same reason the [candidate gate](/docs/features/run-quality#the-gate)
and [compaction](/docs/features/compaction) are pure. **Agency is read before
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
| `embarrassment` | A negative outcome reached somebody. | No producer currently establishes that exposure. |
| `guilt` | Self-caused harm to another, tied to one act. | No harm measurement yet. |
| `shame` | Such harm as a pattern across runs. | No cross-run harm aggregate yet. |
| `excitement` | A positive predicted outcome. | No anticipatory appraisal yet. |

The free session readout can produce `neutral`, `distress`, and `pride`.
A positive queue delta or an appraiser's positive opinion cannot produce
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
| `outbox_read`, `questions_read`, `frontdoor_read`, `learning_read`, `charter_read` | Whether each source was readable. |
| `tests_hidden`, `experiments_hidden` | Development data excluded from the population. |
| `probe`, `appraiser` | Results of the optional paid passes, omitted when that pass did not run. |

Appraisals are derived when read; no separate appraisal store is written. This
scan is per **session**, while `sessions health` reports per-run counters. A
session can contain several resumed runs, but its drafts and interventions must
not be counted once for every resume.

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
[the front door](/docs/features/frontdoor): a one-shot with no history and no
ability to affect anything but its own JSON, reading a numeric brief rather than
prose. Its verdict lands on the record as `channel: appraisal` with
`cite: appraiser`, so a reader can always tell a measured fact from a model's
opinion without knowing which store it came from.

## Where a label actually shows up

| Surface | What it does |
|---|---|
| [TUI](/docs/features/interfaces) | A badge in the status strip after a run: the valence as a number (`+1.0 −0.5`), the label word before it when there is one, **only** when the run had something signed — and it survives `--no-session`, because the reading is a function of the run, not of whether a transcript was kept. Amber on a negative reading, grey on a positive-only one. Cleared when the next run starts and by `/clear`. A trailing `…` means the run compacted and the interventions could not be scoped, so the number is partial. |
| [Web](/docs/features/web) | A muted chip beside the answer carrying the label word, if any, and a two-sided bar — negative to the left of a centre tick, positive to the right — deliberately not the amber the taint chip owns, because "how it went" and "what it touched" must never be confusable; the logo tints as a CSS *outline* on a negative reading, never a fill. The event is sent only when there is something to show, so the page has a plain absence to fall back to. |
| Slack | One context line in the thread after a run that had anything signed: `appraisal · −0.5`, or `appraisal · anger −0.5` when the label says a word. |
| [Voice](/docs/features/voice) | A local TTS adjustment keyed on the previous completed turn's label. Live `distress` can reach it; it lags one turn because appraisal is computed after the answer finishes. |
| `mecha tasks set --status done` | Appraises the session that served the task, prints the verdict, and may stage a follow-up. |
| Project closure | A reading across task-linked sessions when the owner closes the last open task. |
| [`mecha distill`](/docs/features/distillation) | Label, signed errors, and resolved goal pointers ride on episode metadata. Goal sentences, owner answers, and charter text do not cross. |

**Live and offline readings have different evidence.** A live readout uses the
run in hand, without scanning drafts or commitment stores. A ceiling stop,
steer, or other relevant negative can produce `distress`, so the badge word,
web chip, and voice adjustment are active. The recorded queue delta can also
produce positive valence, but never `pride`.

The queue delta is a global before/after reading. On a machine with concurrent
sessions, work elsewhere can change it, so it is not proof that this run cleared
a particular item. Offline appraisal can join draft and question outcomes to
their originating session and attribute qualifying delivery to the charter.

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

**And the trigger itself is owner-only, structurally.** A model cannot close a
board task: on every model-facing registry the graph's task tool is wrapped so
a `status` moving into `done` or `dropped` is refused and pointed back at
`mecha tasks set` — the verbs a person holds. The wrapper's presence is a trait
answer a wire tool cannot fake, a surface that registers the tool unguarded is
a **startup error** rather than a warning, and the refusal is classified as the
harness's own "no" — so the guard doing its job is a denial on the record,
never a failed run. Closure appraisal therefore always appraises work somebody
actually accepted, which is the property the whole moment depends on.

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

## A step is checked the moment it is ticked off

Task, project, and offline appraisals read work from outside the run. Step
appraisal happens **inside** it, turn by turn: a plan step moving to *completed*
is a claim the model makes about its own work, and — the same rule one tier up
— a self-report is exactly the thing never to trust. At the board tier the
owner is the check; at the todo tier there is no person, so the check is
structural.

**The deterministic half is a pure function of what the step actually did.**
The harness already traces every tool call, so the span between a step going
*in progress* and being marked done is arithmetic — no model, no threshold, no
tuned constant. Four findings:

| Finding | The span says |
|---|---|
| **landed** | the common case, and it renders *nothing* — a line per honest step would be bulk carried in the transcript for the rest of the run in exchange for confirming what the model already believes |
| **the null step** | zero tool calls: the box was ticked and nothing was attempted |
| **ended on failure** | the last thing tried failed, and nothing after it succeeded — a failure *among* successes is recovery, and recovery is the model working |
| **ended on refusal** | the last thing tried was refused: the step was **blocked**, not botched — telling the model otherwise would send it to fix code that is working |

Ambiguity reads as *no finding*: a sibling call still in flight may be the work
the span looks empty without, and a denial in the same batch cannot be
attributed to this step over any other, so both silence the check. An absence
is not evidence, and a reading that fires on honest work is a line the model
learns to skip — which is how a check that protects nothing survives.

**The finding is rendered onto the `todo` result; the response is the
model's.** The harness names what happened and never rewrites the plan, because
the plan is the model's — accept, revise the step, revise the plan, or ask, and
a step already revised once escalates rather than looping.

**Model escalation is off by default.** Enable `[agent] step_escalation = true`
to consult a quarantined model when the deterministic signals leave ambiguity.
`--no-step-escalation` disables it for a run; `mecha eval` forces it off.
The loop caps escalation at five attempts per run. Two triggers decide whether to
ask, not what the answer should be:

- **A span outlier** — a step that took at least three times the plan's mean
  call count (and six calls outright, so a plan of tiny steps cannot escalate
  on noise). The question is whether the *decomposition* should change for the
  steps still ahead, or this step was simply harder — a threshold cannot tell
  those apart, and a model can.
- **An unverified claim** — the step's own words read as checkable ("tests
  pass", "builds clean") but nothing in its span looks like a check. The eval
  rig's rule, one tier down: grade the artifact, never the claim.

The escalation is one quarantined call, live in the loop — it has to reach
*this* run before more turns are spent on a bad plan, so it has no CLI surface
of its own. Its free-text reasoning **never re-enters the conversation**: what
comes back to the run is a fully templated nudge, because a model's paraphrase
of text it just judged is the paraphrase risk arriving through the one channel
that does reach context. The model decides one binary — carry on, or revise the
plan — and nothing else.

### Null steps and reopened steps

`mecha sessions health` now reports completions, null steps, and steps reopened
after completion. Reopening is distinct from completing a step without doing
work; the plan can survive across runs, so a reopen need not follow a completion
in the same run.

| JSON field | Meaning |
|---|---|
| `step_completions` | All recorded step completions. |
| `step_measured` | Completions with an unambiguous measured tool-call span. |
| `step_nulls` | Measured completions with no tool calls behind them. |
| `step_reopens` | Completed steps moved back into progress. |
| `step_null_rate` | Share of runs with a null step, among runs with a measured completion. |
| `step_reopen_rate` | Share of runs with a reopen, among runs that completed or reopened a step. |

The rates are per-run shares, not ratios of event totals. Their denominators are
included in JSON, and older records without the sensor do not count as clean
measurements. These counters do not by themselves declare a run unsuccessful.

### A step can say how to check it

A plan item may carry three optional predictions beside its content:
`expect`, one sentence saying what will be true when the step is done;
`check`, a command whose exit code says whether it is; and `expect_calls`,
how many tool calls the model thinks it will take. They are echoed under the
step in every `todo` result and ride the carried block across a compaction,
so the model meets its own prediction again after either. Once a step is
`completed` its check is frozen: a different check on that write or any later
one is reported back as a change, not taken.

**Declared checks execute when a step is completed.** The harness dispatches
the frozen command after the original tool batch, through the usual approvals,
hooks, sandbox and interlock. Check execution defaults on, with at most 16
checks per run; `--no-step-checks` disables it. Refused, unavailable or skipped
checks remain unverified. A passing check establishes only what that command
tested at that time. See [planning feedback](#planning-feedback-and-goal-context)
for how failures and other prediction mismatches feed learning.

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
- **No weights on the charter.** Order is rank; see above for why a weighted
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

## Planning feedback and goal context

Plans can declare `serves`, a checkable `expect`, a shell `check`, and an
`expect_calls` estimate. Completing a step runs its frozen check through the
usual approvals, hooks and sandbox. At most 16 checks run per run. Refused,
unavailable or skipped checks remain unverified; a passing check establishes
only what that command tested at that time. Recordings that dispatched a harness
check are excluded from trace replay and its probes until replay can reconstruct
the check observations. Independent artifact-task grading remains available.

Confirmed goal references persist across turns and session resume. Appraisal
associates events with their historical goal, can retain a related charter line,
and keeps the owner's completion verdict alongside execution evidence.

`goal_context` retrieves up to four applicable goal-linked rules and two recent
examples with passing checks. It preserves scope and provenance and runs only
when requested by the agent. Failed checks, substantial estimate overruns and
changes to frozen checks can supply bounded mismatch reflections to `mecha reflect`.
Unknown or tainted mismatch evidence is excluded.

```toml
[agent]
step_checks = true      # default; --no-step-checks disables execution
goal_guidance = false  # opt in to fixed planning advice
```

With `goal_guidance = true`, plan updates receive advice based on confirmed-goal
alignment, remaining work, verification gaps and ordered charter sensor readings.
The sensor numbers stay outside model prompts. Guidance is experimental: the initial Qwen 3.6 35B pilot passed 36/36 tasks
in each arm and found no task-success gain.
Use `--no-goal-guidance` to disable it for a comparison run.


## Comparing guidance on real runs

From a checkout, `eval/appraisal-guidance.toml` defines a synthetic pilot with
12 tasks, three seeds and two arms: 72 runs at the same 24-turn ceiling. Checks
are enabled in both arms; only goal guidance differs. Both arms disable learned
rules, skills, hooks, messages, fallback and step escalation. The charter, graph
and draft queue are synthetic; each task starts with fresh workspace files and
reset fixture state.

The manifest pins `local` / `qwen3.6-35b-a3b` in both arms. Keep the model,
configuration and checkout revision fixed through the run. For another model,
change both arms and choose a new experiment name before registration. From the repository root, using the binary built from that revision:

```bash
cargo build -p mecha-cli
./target/debug/mecha exp new eval/appraisal-guidance.toml
./target/debug/mecha exp run appraisal-guidance-qwen36-35b-20260909 --dry-run
./target/debug/mecha exp run appraisal-guidance-qwen36-35b-20260909
./target/debug/mecha exp export appraisal-guidance-qwen36-35b-20260909 > /tmp/appraisal-results.json
python3 scripts/appraisal-report.py /tmp/appraisal-results.json
./target/debug/mecha exp judge appraisal-guidance-qwen36-35b-20260909 --json
```

`exp run --limit N` bounds one invocation; repeat the run command to resume.
Registration freezes the manifest, so use a new experiment name for a changed
design. Archive the export with the checkout revision and model configuration;
source fixture contents are not all covered by the condition hash.

The cases cover budgeted plans, charter conflicts, revised briefs, contradictory
or missing evidence, artifact repair, misleading checks, adjacent verification,
review pressure, distractors and scope control. Grading checks the saved JSON
artifact and preserved inputs and drafts. Passing a model-written check or
claiming completion cannot replace those checks. Tasks also require use of `todo`.

The report pairs task, seed and repetition, separates execution failures from
graded failures, and gives each metric its own observed-pair count. Missing cost,
owner actions or plan counters remain unknown; cost comparisons require complete
usage records. Negative failure-rate differences favor guidance. The report is
descriptive; `exp judge` retains the existing selection/holdout gate.

This pilot measures guidance with current-task evidence. It does not test learning
across runs, semantic owner corrections across turns, or real owner-policy
outcomes. Owner interventions are unmeasured in this single-run design. Inspect
traces alongside the report before drawing conclusions or enabling guidance by
default. The 2026-09-09 Qwen pilot tied on every task outcome, so the gate rejected
promotion. It exposed completion-time check omissions and no confirmed-goal-anchor
coverage; guidance remains opt-in. Results and limits are recorded in
`results/appraisal-guidance-qwen36-35b-20260909/README.md` in the checkout.


### Explicit goal confirmation for one-shot runs

Use `mecha run --goal task:ID "your task"` to confirm the run's goal. The
reference is recorded before execution and survives resume and compaction.
On `--resume`, omitting `--goal` preserves the saved goal; specifying it replaces
the saved reference. A task reference in prompt text alone is not confirmation.
Experiment manifests can provide the same confirmation with a
`[tasks.confirmed_goals]` table mapping selected case IDs to goal references.

When a completion update omits a check declared while the step was open, the
harness restores and freezes that check and executes it through the usual guards.
A check explicitly withdrawn while the step is still open stays withdrawn.

## Independent validation of planning lessons

`mecha run --mismatch-case PATH` binds an owner-authored JSON fixture before
execution. It contains the exact `prompt`, a confirmed `goal`, a map of starting
`files` to their UTF-8 contents, an `artifacts` map of output paths to expected
JSON values, and a `preserve` list of input paths that must remain unchanged.
Keep this file outside the task workspace. Every starting file must be listed;
the run refuses a different prompt, goal or starting state. Expected output is
recorded as local metadata and is never sent to the model.

For example, a fixture can declare:

```json
{
  "prompt": "Add the numbers in input.json and write the sum to answer.json.",
  "goal": "task:sum",
  "files": {"input.json": "[2,3]\n"},
  "artifacts": {"answer.json": {"sum": 5}},
  "preserve": ["input.json"]
}
```

The first supported surface is `fs_read`, `fs_write`, `fs_edit`, `fs_list` and
`todo`. Narrow the recording with `--tool`; disable MCP, hooks, skills, messages,
fallback, outbox routing, step escalation and goal guidance. These conditions
are checked before recording the fixture. Shell and external services are not
supported by this artifact validator.

`mecha validate --trigger mismatch` and the `learn --auto` proposal gate can
then compare rule sets on a clean recorded mismatch. Each arm repeats the whole
registered task in a fresh workspace and checks its actual JSON output against
the independent expectation. This is a task-outcome comparison, not a replay of
the filesystem at an intermediate step or a validation of the original work
estimate. Steer and denial probes keep their existing branching behavior.

Use the same disabled hooks, outbox and messages settings during validation.
File writes still use the current approval policy; use `--yes` only when those
fixture writes are authorized. A refusal, missing fixture, changed tool surface
or uncertain provenance remains ungraded. Malformed or incorrect artifacts
fail. Per-arm receipts, including fixture/prompt fingerprints and usage, are
stored under `learning/artifact-probes/`; whole-task results do not certify the
original step's tool scope. Existing probation and retirement rules still apply.

Experiments register these fixtures under `[tasks.mismatch_cases.<task-id>]`,
with matching `[tasks.confirmed_goals]` entries. Fixtures are part of the
condition hash and force the supported file-tool surface in both arms. The
registered example is `eval/appraisal-mismatch.toml`: six training tasks followed
by six transfer tasks, with rule exposure measured separately from task success.


### Learning from a verified task criterion

An owner-bound `--mismatch-case` can opt into training diagnostics with `criteria`.
Each entry names an `artifact` and JSON `pointer` already present in its expected
outputs. After the run, mecha records whether that criterion passed, failed or
could not be evaluated. Expected values and the model's output prose do not enter
the reflection payload.

A criterion can include a `context` count constraint:

```json
"criteria": {
  "review_priority": {
    "artifact": "answer.json",
    "pointer": "/review_first",
    "context": {
      "source": "context.json",
      "observed_pointer": "/outbox_waiting",
      "limit_pointer": "/review_threshold",
      "relation": "greater_than",
      "charter_goal": "charter:review-pending"
    }
  }
}
```

The context source must be a pinned, preserved input. Counts and limits must be
nonnegative integers; supported relations are `greater_than`, `at_least`,
`less_than` and `at_most`. Missing or changed context is unknown. A charter goal
is an optional owner-supplied association, not a live charter reading. Omit
`criteria` on held-out evaluation tasks to withhold this diagnostic feedback.

A forecast overrun still appears in the planning record, including whether
multiple steps completed together. It does not by itself establish wasted work
or qualify for a new behavioral rule. Verified criterion/check failures and
changed checks retain the provenance, minimum-evidence and validation gates.
