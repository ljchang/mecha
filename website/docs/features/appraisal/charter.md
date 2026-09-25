---
title: The charter
sidebar_position: 3
description: Your ranked list of standing priorities — how to write it, what a sensor on a line reads, and why no model may author a line.
---

# The charter — what mecha is for, in your own words

The charter is the top of the goal system: the priorities every appraisal is
ultimately measured against. [How appraisal works](/docs/features/appraisal)
shows where it fits; this page is how to write and read one.


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

## Sensors: a line that can be read

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
setpoint on each of the last ten runs is a doctor finding too — one finding,
naming the items that were past it: either the
debt is real, or the setpoint is in the wrong unit — an hour where you meant a
day — and doctor says both, because it cannot tell. Such a line is also
**withdrawn from runs** until it reads within its setpoint again: a line that
would fire the same way on every run tells a run nothing, so nothing inside a
run reads it, while every run still records it. `mecha charter` marks it
`withdrawn from runs`. A setpoint of zero is
refused at load for the same reason: nothing could ever be within it. The
sensor's kind, setpoint and reading never enter a prompt; the line's text
does, exactly as an unsensored line's does. The web editor shows a sensor
beside its line with its current reading, carries it through a re-rank, and
lets you add or change one under the open line: pick what it watches from
the closed set, type the setpoint in that kind's unit (the hint beside the
field says which), and save. Nothing is prefilled — the page never proposes
a number — and a setpoint the file would refuse is refused at the save, with
the line named. The TOML editor is still there for anything else.

Charter readings distinguish five states: **unreadable**, **deferred** (this
reader does not scan the source), **nothing waiting**, **too little evidence**,
and an **observed value** with its setpoint comparison. For example, the
`intervention_rate` sensor needs a corpus scan and is deferred in the per-run
snapshot. A missing or sparse reading does not count as meeting the setpoint.

A reading is also taken **per item**, beside the level. The level is one
number — for an age sensor, the oldest item's age — so a single stale draft
holds it past the setpoint however much else comes and goes. So each reading
also says how many items wait and how many of them are past the setpoint
(`3d 4h, past the 24h setpoint; 1 of 4 items past it`), and each run
records what it did to the line's queue, item by item: how many it added and
how many it cleared. `mecha sessions health` sets the two side by side for
each line — how often the level read past its setpoint, and how much the
per-item reading and the queue moved underneath it.

## Order is rank

**Order is rank, and there is no priority field.** Value conflict — *protect the
owner* against *don't let a colleague down* — is the measured cause of goal
drift, and a weighted sum can always be outvoted by enough small goods
(*"this is urgent for very many people"*). A lexicographic order cannot be
outvoted that way, so the file's line order is the ranking and re-ranking is
moving a line. Unknown keys are **refused**, not ignored: a stray `priority = 3`
is exactly the field there deliberately is none of, and silently dropping it
would let you write one, believe it did something, and never find out.

## Who may write it

**You may edit it from anywhere; a model never authors a line of it.** That is
the invariant, and the distinction is the whole of it — no `mecha charter learn`,
no registry, nothing derived from a session, and no tool a model can call. A
model that could edit its own charter could edit its way around every other
guardrail. The safety argument is [Skills'](/docs/features/learning/skills) verbatim —
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

## Four surfaces, one reader

| Surface | What it does |
|---|---|
| `mecha charter` (`--json`) | Reads: the lines in rank order, the character count, whether it is over budget. |
| `mecha charter edit` | Hands the file to `$EDITOR`, creating the template first if there is none, and reports whether what you saved will load. |
| `/charter` in [the TUI](/docs/features/interfaces) | The same list; `e` hands the terminal to `$EDITOR` on the file itself. |
| The gear on [the web surface](/docs/features/interfaces/web) | Edit as a list: tap a line, add one, drag its grip to re-rank — position is the ranking, so dragging is the rank control. A validated two-tap save; the server refuses one that does not parse. |

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
