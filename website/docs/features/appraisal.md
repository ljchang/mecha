---
title: Goals and appraisal
sidebar_position: 21.5
description: A charter saying what mecha is for, a signed error against it, and a label derived from the record — never reported by the model.
---

# Goals and appraisal

Every evaluative signal mecha had was either **a human intervening** or **a
counter crossing a threshold**. `reflect` mines four ways of saying a person
stepped in; the harness gate scores six metrics whose docstring makes
lower-is-better an invariant. Two consequences followed, and neither was
visible from inside any one subsystem:

- **Every signal was negative.** There was no channel through which a run
  could be recorded as having gone *well*, so nothing could prioritise between
  two runs that both merely avoided harm.
- **Every signal was exogenous.** Four of the five loops could not start until
  the world acted first. mecha could not notice unprompted that something went
  badly against what it is for, because nothing represented what it is for.

This is the half that was missing: a written statement of standing priorities,
a reference every piece of work can cite, and a **signed** error against it.

```bash
mecha charter                        # the standing priorities, as a run sees them
mecha sessions appraise --days 30    # how runs went against what they were for
mecha sessions appraise --probe      # the paid pass that fills `controllable`
mecha sessions appraise --appraise   # the quarantined appraiser's second opinion
```

:::note Observation, mostly
Almost nothing consumes an appraisal yet, and that is deliberate — the number
worth reading first is **how many runs come back with no label at all**. On the
120-session corpus this was built against, 119 came back `neutral`. See
[the finding](#the-finding-most-runs-have-no-label) before building on it.
:::

## When appraisal happens

Four moments, three timescales. Each one reads only records that already
exist, and none of them writes an appraisal store — an appraisal is derived on
read, so a change to the derivation replays over the whole corpus instead of
being lost with it.

| Moment | Trigger | What runs | A model in the path? |
|---|---|---|---|
| **A plan step is ticked off** — turn by turn, inside the run | the model marks a `todo` item completed | a deterministic reading of what the step actually did — [below](#a-step-is-checked-the-moment-it-is-ticked-off) | one quarantined call, only when a pre-filter finds real ambiguity |
| **A run finishes** | every run | the free per-run readout, a pure function over the run's own records — it feeds [the badge, the tint and the voice nudge](#where-a-label-actually-shows-up) | never |
| **The owner closes a board task** | `mecha tasks set --status done` (or `dropped`) | [one closure appraisal, ever](#closing-a-task-appraises-it); a disappointed `done` may stage one follow-up | never |
| **On demand, offline** | `mecha sessions appraise` | the free scan over transcripts, outbox and outcome records; [`--probe` and `--appraise`](#the-two-paid-passes) are the paid opt-ins | only behind those two flags |

**Nothing is scheduled.** There is no nightly job: the paid passes run when
you run them, and the design's "periodic" moment is exactly the free scan
above. The asymmetry across the table is the design — the checks that fire
often are deterministic and free; a model appears only where the free signals
are genuinely ambiguous, and then always as a quarantined one-shot.

[Boredom](#boredom-naming-an-approach-that-has-stopped-teaching-the-run-anything)
also fires inside the run, but it is a different kind of object — a mood, a
statement about a trend, named while there is still something to do about it —
and it is covered at the end of this page.

## What appraisal is for

The record exists so that five things can happen. Three already do:

- **You can see how a run went without asking the model how it felt.** The
  [readout surfaces](#where-a-label-actually-shows-up) and
  `mecha sessions appraise` are display over derived facts — there is no
  self-report anywhere in the path.
- **Closing a task can turn residue into work.** A closure the owner accepted
  but the record says went badly is invisible to every counter, and it is
  precisely the closure most likely to have one task's worth of residue —
  [the follow-up](#closing-a-task-appraises-it) is that signal acting.
- **Episodes carry how-it-went into memory.** The label and the signed errors
  ride on a [distilled episode's](/docs/features/distillation) `meta`, beside —
  not inside — its content, so the knowledge graph can weight what a session
  said by how the session went.

Two more are what the *sign* makes possible, and both deliberately wait on
[the finding](#the-finding-most-runs-have-no-label) below:

- **A positive half for learning.** Today `learn` consolidates the writing
  domain from edited drafts only — it can learn what displeased and never what
  landed. A draft **sent unchanged** is a positive signal, authored by the
  owner rather than the agent, recorded for the whole life of the outbox with
  nothing reading it. The sign is what lets it count.
- **Priority for the self-improvement loop's paid replays.** Replay minutes are
  the scarce resource in [the harness loop](/docs/features/run-quality), and
  the appraisal is designed to be the priority function that spends them — a
  *priority* function, never an objective one. Nothing optimises toward the
  label, and `visible` is computed exposure rather than a feeling, precisely so
  this never becomes *the agent optimises to feel good*.

The pattern across all five is the same one the homeostat and boredom shipped
under: **the sensor ships first and earns a behavioural consumer later**,
rather than backing into one under time pressure. Affect may eventually narrow
a disposition; it may never loosen one; and today it does not reach the model
at all.

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
```

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
itself only ever reads.

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

There is a **2,000-character budget**, checked by `mecha doctor` and shown in
every surface. It is not enforced — over budget is a finding, not a refusal —
because the cost is prefix bytes on every request, which is a thing to be told
about rather than stopped for.

## What a run is for

A `GoalRef` is a **pointer, never a copy**, and renders on the wire as
`kind:id`:

| Kind | Points at |
|---|---|
| `charter:<line-id>` | A standing commitment — a `[[line]]`'s own `id`. |
| `task:<uid>` | A task on [the graph's board](/docs/reference/cli#tasks), by the graph's uid. |
| `setpoint:<name>` | A homeostatic setpoint. Named so the wire format survives its arrival; no store yet. |

A flat string rather than a nested object because the **model** writes it: it is
one field on the `todo` tool's schema, and malformed arguments are a metric the
harness grades models on. One string is harder to get wrong than
`{"kind": …, "id": …}`.

```
todo(items=[…], serves="task-1a2b3c4d")
```

The plan echoes it above the list, so `serves task-1a2b3c4d` survives into the
transcript and across compaction — which is how an appraisal built later knows
what the run was for. Reading a ref back has two policies on purpose: **from the
model** a malformed ref is an error reported through the tool result, because
the model can fix it and a silently dropped field leaves a plan claiming to
serve nothing; **from a record** an unknown kind degrades to *no reference*,
because transcripts are append-only and may have been written by a newer binary.

A run that names no goal appraises with none. That is recorded rather than
guessed — every record cites the tier above it, and a run with no tier above it
is a fact about the run, not a reason to lose its errors.

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

Nothing consumes the number yet. It is recorded so the corpus exists before
anything is built on it, the same way the homeostat and boredom both shipped.

## The appraisal record

One `Appraisal` per session or per closed task: what was live, the conditions,
a list of **signed** errors, and a label derived from them.

Each `GoalError` is one signed error on one goal, across six dimensions:

| Dimension | What it holds |
|---|---|
| `goal` | What it was an error *against*, or nothing. |
| `channel` | Which of the five signal paths it arrived on. |
| `sign` | Negative is worse. **The whole point of the record** — the harness gate's metrics are monotone cost by deliberate constraint, so nothing there can represent a run that went well. |
| `agency` | Who caused it: `self`, `owner`, `other`, `world`. |
| `visible` | Did the outcome reach anyone. A computed fact about exposure, never a feeling the model announces — which is what stops this becoming *the agent optimises to feel good*. |
| `controllable` | Could it have gone otherwise? Unfilled until a counterfactual probe says. |
| `cite` | **A pointer, never prose** — a turn index, a draft id, a counter name, a setpoint name. |

The five channels are named rather than merged, because five loops had already
converged on one word for *what this was decided from* without converging on the
concept:

| Channel | Source |
|---|---|
| `intervention` | A human steered, denied, or came back to correct. |
| `edit` | An outbox draft was edited before it went — **or sent unchanged**, which is the one channel in this system that can say something went well, and was recorded for the whole life of the outbox with nothing reading it. |
| `counter` | A counter on [the run's own record](/docs/features/run-quality). |
| `setpoint` | A homeostatic variable outside the range it is kept in. |
| `appraisal` | The agent's own, from the quarantined pass. |

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
| `neutral` | Nothing the derivation can name. | ✅ the common answer |
| `anger` | Negative, caused by something with no address here — a 429, an MCP server, a machine under load. | ✅ |
| `regret` | Negative, self-caused, and an alternative existed. | ✅ probe only |
| `disappointment` | Negative, and no alternative existed. | ✅ probe only |
| `frustration` | Repeated negative error on one goal with no progress between. | ✅ probe only |
| `embarrassment` | Negative, and it reached somebody. | ❌ no producer |
| `guilt` | Self-caused, harmed another, attaching to one act. | ❌ nothing computes harm |
| `shame` | The same, attaching to a *pattern* across runs. | ❌ needs an aggregate |
| `pride` | Positive, self-caused, against a charter line rather than a task. | ❌ needs charter closure |
| `excitement` | A positive *predicted* error. | ❌ needs anticipatory appraisal |

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
*"N of the ten variants"* line is derived from it — that line shipped stale as a
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

```text
118 session(s) appraised, of 140 read

  label
    anger                1  (1%)
    neutral            117  (99%)

  99% carry no label — 5 of the 10 `Affect` variants need a charter, a notion of harm, a cross-run view, a prediction, or an exposure producer

  signed errors, by channel
    counter             14
    intervention        33
    of which +ve         0  — the only channel that can say a run went well
```

Nothing is stored. Appraisals are derived on the spot from the transcripts, the
outbox and each run's own outcome record, and `--json` emits the same figures
for a script. Three reporting rules to notice, because each is the difference
between an absence and a zero:

- *"The outbox could not be read, so the edit channel is missing — not empty"*
  is printed whenever the store failed to open, and printed **before** the early
  return, because a store that could not be read is a fact about this run
  whether or not anything was left to appraise.
- The same rule covers the transcripts themselves: **sessions read and sessions
  unreadable are disjoint counts**, carried on `appraise`, `stats` and `health`
  alike, and an unreadable transcript is a `mecha doctor` finding rather than a
  silently smaller denominator. An instrument must not eat its own findings.
- Probe and appraiser statistics are **absent** from `--json` when the flag did
  not run, rather than zero. *"Nothing was probed"* and *"probed and found
  nothing"* are opposite findings.

The walk is per **session**, not per run. An intervention carries a message
index with nothing saying which run held it, and an outbox item records the
session that drafted it — so attributing either per-run would multiply both
channels by the number of times the session was resumed.

### The finding: most runs have no label

On the corpus this was built against, **119 of 120 sessions labelled
`neutral`**, and the reason is structural rather than a tuning problem. The free
readout can only ever say *neutral* or *anger*: every negative it assembles is
either self- or owner-caused with `controllable` unfilled (which reduces to
neutral) or a ceiling nobody here caused (anger); and no counter kind fires
twice in one session, so frustration's repetition cannot occur.

That is the measurement the rung exists to produce, learned cheaply here rather
than after something was built on it. The alternative — inventing precedence
until every run gets an interesting word — manufactures exactly the signal this
was meant to test for.

## The two paid passes

Both are off by default, both are independent of each other, and both have their
own ceiling.

### `--probe` — the counterfactual

```bash
mecha sessions appraise --probe --max-probes 25
```

Every intervention drives one replay of the recorded run **without** the
steering text, to see whether the run got there anyway. That is what fills
`controllable`, the field 100% of the corpus's labels were stuck on.

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
| [TUI](/docs/features/interfaces) | A badge in the status strip after a run, **only** when the label is not neutral — and it survives `--no-session`, because the label is a function of the run, not of whether a transcript was kept. Cleared when the next run starts and by `/clear`. |
| [Web](/docs/features/web) | A muted affect chip beside the answer — deliberately not the amber the taint chip owns, because "how it went" and "what it touched" must never be confusable — and the logo tints as a CSS *outline*, never a fill. The event is sent only for a non-neutral label, so the page has a plain absence to fall back to rather than a stream of `"neutral"` saying nothing. |
| [Voice](/docs/features/voice) | A per-answer weight nudge on the local TTS. It **lags one turn by construction** — the label is a function of a *finished* run, so a call hears the previous turn's mood. |
| `mecha tasks set --status done` | Appraises the session that served the task, prints the verdict, and may stage a follow-up. |
| [`mecha distill`](/docs/features/distillation) | The label and the goal errors ride on an episode's `meta`, beside — not inside — its content. |

**A compacted run reads as neutral outright.** Compaction rewrites the message
list in place, so the index marking where the run began no longer names its own
starting point, and there is no way to recover the boundary. Dropping just the
interventions is not the safe direction it looks like: the derivation reduces
magnitude-first, so losing a steer *un-masks* a smaller error and produces a
**louder** reading. Given how strongly compaction correlates with long, hard
runs, that would make the readout predominantly mean "this run compacted".

### Closing a task appraises it

```text
mecha's appraisal of task-1a2b3c4d: Anger (0 positive, 1 negative signal)
```

It counts and never quotes — the line is the label and two tallies, because the
signals themselves are pointers rather than prose. Only the transition *into*
`done` or `dropped` triggers it, and only once. Two conditions gate the
follow-up task, both load-bearing:

- **The label, not the raw signs.** Re-deriving a threshold over the signed
  errors here would be a second, less-tested copy of the reduction the label
  already is — and it would fire on almost every closure, since a negative
  signal appears on 119 of 120 sessions. Neutral must never stage a follow-up
  nobody asked for.
- **`done` only, never `dropped`.** The trigger is the owner *accepting* the
  work — a disappointed closure they took anyway. A dropped closure is the owner
  declining it, so proposing a follow-up there would override a decision they
  just made. (This one was found on review: a `MaxTurns` run the owner gave up
  on got a "Revisit" task put straight back on the board.)

Staging `anger` is a decision rather than an accident of "non-neutral". It
stages *work*, not blame: today the only free path to anger on a closure is a
ceiling stop, and a ceiling-cut run the owner accepted as done anyway is
precisely the closure most likely to have residue worth one task — the part the
ceiling cut off.

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

## A step is checked the moment it is ticked off

The three moments above all appraise finished work from outside the run. The
fourth happens **inside** it, turn by turn: a plan step moving to *completed*
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

**A model is consulted only where the free signals cannot decide, and the
pre-filter never decides the answer — only whether to ask.** Two triggers, both
comparisons rather than facts about one span:

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

## What is deliberately not here

- **No self-reported feeling.** There is no field a model can write a label
  into, and no path by which one reaches the system prompt as free text.
- **No optimisation against the label.** `visible` is computed exposure, not a
  feeling, precisely so nothing here becomes *the agent optimises to feel good*.
- **No schedule.** Both paid passes are commands you run, not jobs that run
  themselves — a nightly pass that quietly spent replays and model calls would
  be a bill discovered rather than decided.
- **No weights on the charter.** Order is rank; see above for why a weighted
  sum is the thing an injection can outvote.
- **No model-authored charter line, at any privilege level.** You edit it from
  wherever you like; nothing that is not a person writes to it.
- **No store for appraisals yet.** They are derived on read from records that
  already exist, which means a change to the derivation replays over the whole
  corpus instead of being lost with it.
- **Affect may only narrow a disposition, never loosen one** — and today
  nothing reads it as far as the model at all. The sensors ship first and earn a
  behavioural consumer later, deliberately, rather than backing into one under
  time pressure.
