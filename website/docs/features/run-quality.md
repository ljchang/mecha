---
title: Run quality
sidebar_position: 21
description: The corpus of how runs went rather than what they cost — and the detect, diagnose, measure, gate loop built on top of it.
---

# Run quality

Every finished run writes one row saying **how it went**, as distinct from what
it said or what it cost. Read across a few hundred sessions those rows say what
normal looks like, and when it stopped.

```bash
mecha sessions health --days 30      # the human view
mecha doctor                         # the alarm, when a rate crosses a threshold
mecha diagnose                       # one proposed change, with a prediction
mecha eval --ab-config max_turns=40  # the measurement that would falsify it
mecha harness ruminate               # all four, nightly, on a timer
```

Those four commands are one loop — **detect → diagnose → measure → gate** — and
the reason it exists is that nothing else in mecha could see a harness problem.
`reflect` mines the moments a *human* stepped in, so a run that quietly failed a
third of its tool calls produced no intervention, no reflection, and nothing
downstream ever heard about it. The corpus is the sensor that was missing.

## The outcome record

`Record::Outcome(RunStats)` lands one line per finished run in the session
transcript, written by every front-end — `run`, `chat`, the TUI, Slack, a
trigger.

The gap it closed: `RunOutcome` carries fifteen fields and the transcript kept
two of them, so an interactive run was measurably **less** observable than an
unattended one, whose trigger ledger recorded the rest. The signal was already
computed and thrown away at the end of every run a human was watching.

| Field | What it says |
|---|---|
| `turns` | model turns spent |
| `usage`, `cost_usd`, `usage_complete` | what it cost, and whether that is a measurement or a lower bound |
| `stop_cause` | why the loop stopped — the single most informative field, and invisible in the answer text |
| `exhausted` | a budget was reached |
| `ended_on_failed_call` | the model stopped of its own accord with its last call failed |
| `tool_calls`, `tool_errors`, `tool_denied`, `tool_staged` | calls attempted, and how they went |
| `malformed_tool_args` | arguments the model produced that did not parse |
| `blocked_sends` | sends the trifecta interlock refused |
| `compactions` | summaries taken |
| `taint` | what had entered the conversation by the end |

Two properties make this usable as an input to automated grading, and both are
structural rather than conventional:

- **Every field is a deterministic count**, and none is derived from the
  *content* of a tool result. A counter carries no instructions, so a corpus of
  them cannot be an injection surface the way excerpts would be. This is the
  same shape as [the front door's](/docs/features/frontdoor) rule that the privileged run
  sees the extraction and never the prose.
- **A denial is not a failure.** `tool_errors` counts the environment refusing;
  `tool_denied` counts a human or a policy refusing, which is the harness
  working. Averaging the two together makes a read-only run look broken for
  doing exactly what it was told to do — and every rate the doctor thresholds on
  and the gate judges against would carry "the harness working" inside it.

`Summary` still records cost, and this is a second record rather than more
fields on it because the audience is different: cost is for a person reading
`sessions show`, and this is for a machine reading a thousand sessions at once.

## Reading it back

`mecha sessions health` puts the rows side by side. Deliberately separate from
`mecha sessions stats`, which answers what runs *cost* — different question,
different units, different audience.

```bash
mecha sessions health              # everything the store holds
mecha sessions health --days 30    # a window
mecha sessions health -n 200       # bounded by session count
mecha sessions health --json       # machine output
```

```text
412 run(s) across 188 session(s), last 30 day(s)

  stop cause      completed 380 · max_turns 21 · loop 6 · interrupted 5
  ended on a failed call   17 (4.1%)
  tool calls      3162 · errors 233 (7.4%) · denied 41 · staged 12
  malformed args 9 · blocked sends 2 · compactions 61
```

Four rules in the reader, each of which is a bug if undone:

- **It reads the transcripts; there is no ledger file.** The transcript already
  holds the rows, written by the process that produced them. A ledger beside it
  would be faster and would be a second source of truth that can disagree with
  the first — the same reasoning that has the TUI read a trigger's last answer
  back from the session record instead of caching it.
- **Every scan is bounded** — a session cap, an optional cutoff. A reader that
  must read the whole store before it answers is one nobody runs. `doctor`'s
  constraint (one pass, no network, no model) is the bar.
- **A rate over a zero denominator is `None`, never zero**, and prints as `—`.
  "Nothing went wrong" and "nothing happened" are different answers, and
  printing them the same way is how a component that stopped working reads as
  healthy.
- **Rates split by model.** A corpus spanning two models has no single tool
  error rate worth quoting: the blend is true and useless, and a threshold on it
  fires for the wrong model. Runs are attributed from the `config` records
  rather than the session header, so a mid-session model switch lands in the
  right slice.

The module counts and never judges. What counts as a bad rate depends on what
the run was for, so the thresholds live with the reader that acts on them.

## What the doctor does with it

[`mecha doctor`](/docs/reference/cli#doctor) grew a population view to sit
beside its incident checks. Over the last 200 sessions, per model, with a floor
of 20 runs before any rate is reported:

| Finding | Threshold |
|---|---|
| a model finishing runs on a failed tool call | 20% of runs |
| a model failing its tool calls | 25%, and at least 20 calls |
| the harness cutting runs short | 25% of runs |

The thresholds are deliberately **high**. Rule-based evaluators are measured to
under-report success, and a doctor that cries wolf stops being read.
"Cut short" is `max_turns`, an output-token or cost budget, the loop guard, or
a run that produced no output at all. `interrupted` is deliberately **not** in
that set: a person pressing Ctrl-C is the system working, and counting it would
make an attentive user look like a problem. The remedy on each is `mecha sessions health --days 30` — reading, not
fixing, because what to change is in the transcript and doctor never decides
that.

Triggers get the same treatment one layer down, because an unattended run has
nobody watching it fail — the briefing still arrives and the ledger still says
`ok`:

- **A trigger failing a third of its tool calls** across its last five runs,
  silent below ten calls because a rate over three of them is noise. The trigger
  ledger now records `tool_calls`, `tool_errors` and `ended_on_failed_call` per
  run so this is visible at all.
- **A trigger whose most recent run succeeded having done nothing.** The rate
  check cannot see this one — a rate over zero calls is undefined rather than
  bad — so a trigger that made thirty calls a morning and now makes none is
  silent in every other signal. Measured against the trigger's *own* earlier
  runs and never an absolute floor, or a prompt that legitimately needs no tools
  would read as the broken one. Suppressed when the run also errored, because
  that already has a finding, and two findings for one fact leave neither
  meaning anything.

## The diagnostic stage

`detect` finds that something is wrong and the gate decides whether a fix
helped. Neither authors the fix — that step is an inference, so
`mecha diagnose` is where a model belongs, and it is the only place in this loop
one appears.

```bash
mecha diagnose                 # the model with the most recorded runs
mecha diagnose --model qwen3-moe --days 14
mecha diagnose --dry-run       # print the brief and stop, paying nothing
```

**A model is safe here because being wrong is cheap.** Automated failure
attribution measures at 53.5% for naming the responsible agent and 14.2% for
pinpointing the failing step — some methods below random. A diagnostician will
usually be wrong. Every proposal therefore carries a falsifiable prediction and
nothing is accepted until a measurement it did not run confirms it, so a bad
diagnosis costs one replay. That property does not hold at the accept gate,
which is why there is no model there.

Two rules are structural rather than instructed:

- **The brief is built from counters, not content.** `Evidence` holds numbers
  and doctor's findings — machine-authored text, written by this program. There
  is deliberately no field for a transcript excerpt and no argument that adds
  one.
- **The proposal never quotes its evidence.** The diagnostician runs read-only
  with the web tools and may read the source and these docs; a proposal that
  reproduces **eight consecutive words** from anything it read is refused. An
  instruction lifted from a page cannot survive that; a conclusion drawn from
  one can. Eight because shorter runs collide on ordinary technical prose, and a
  check that fires on honest proposals gets turned off and protects nothing.

The output is a typed block, reasoning first and the fields last — constrained
decoding degrades reasoning when the answer precedes the thinking:

```text
── proposal ──
class:     Config
change:    max_turns=40
predicts:  lower CutShort
because:   runs are hitting the ceiling rather than finishing

nothing to do here yet — measure it:
  mecha eval --ab-config max_turns=40 eval/cases.jsonl
```

**It proposes; it does not measure and does not apply.** Running the arms costs
a real model run per case per arm, so making it automatic would put an hour of
inference behind a command whose output is a suggestion. The printed command is
shell-quoted, because `change` is model-authored and this line exists to be
pasted.

Declining to propose is a legitimate answer and is never coerced into a change —
a diagnostician that always proposes is optimizing for proposal frequency, which
is a named failure mode. A block missing its class or metric parses as
*nothing*, because a proposal that cannot be falsified must not enter the gate.

## The gate

`candidate.rs` decides what happens to a proposed change. It is **pure** for the
same reason [`compact.rs`](/docs/features/compaction) is: getting it wrong is silent — a
rule that scores well ships and rides in every future prompt — so it is
unit-tested rather than trialled.

It takes two arms' worth of outcomes and the prediction that was made *before*
either was measured.

**Every metric is phrased as a cost, so lower is better everywhere.** Mixed
polarity is how a comparison inverts silently.

| Metric | What it costs |
|---|---|
| `ended_on_failed_call` | runs that finished with their last tool call failed |
| `tool_error_rate` | share of attempted calls the environment refused |
| `cut_short` | runs the harness ended rather than the model finishing |
| `compactions` | summaries taken |
| `turns` | turns spent |
| `malformed_args` | arguments that did not parse |

Seven decisions carry it:

- **Paired by episode, then split.** Episodes differ from each other far more
  than arms do, so an unpaired comparison measures which episodes landed where.
  Selection happens on one slice and the winner is confirmed on a **holdout it
  was never chosen on** — picking the best of N on the episodes that justify it
  is a multiple-comparisons trap that looks *better* the more it overfits. The
  split is a hash of the episode id, never random: a rerun that resplits is a
  holdout that means nothing.
- **The work guardrail outranks the score.** A change that improves its metric
  while tool calls fall below 75% of the baseline is rejected, not ranked.
  "Fewer errors" is trivially achieved by attempting less — the null run, and
  the reward-hacking result (METR measured o3 gaming its own scorer on 30.4% of
  RE-Bench runs). For the same reason a run that made **no calls** is neutral on
  the error-rate metric rather than perfect.
- **Thin evidence proposes; it never rejects.** An absence of evidence is not
  evidence of harm. The floors are low on purpose — 8 paired episodes to select,
  4 to confirm — because a replay corpus costs a real model run per episode per
  arm, and a floor set where the statistics would like it is a floor that stops
  the loop running at all. The holdout does the work a larger sample would.
- **An episode that ran in only one arm is dropped.** Not a tie and not a loss:
  scoring it either way lets a candidate that dies on the hard episodes look
  good on the ones it survived.
- **Counts, not a significance test.** With a few dozen episodes the noise is
  the model's sampling rather than the measurement, and the answer to that is
  repetition (pass^k), not a p-value over one sample. The raw win/loss/tie
  counts ride on the judgement so a human sees what it was decided from.
- **Two currencies, one gate.** The same judgement grades anything that can name
  an episode and produce a cost: replayed sessions on their outcome counters
  (did the *harness* go better) and eval cases on whether the case **passed**.
  The second is the content-sensitive arm a prose change needs, because replay
  holds tool results fixed and cannot see a change in what the model said. One
  gate, so the holdout and the guardrails cannot drift apart between them.
- **`architecture` and `security` changes reach a person however well they
  scored.** The standing recommendation is that `security` is never proposed at
  all — a loop that can argue for widening its own confinement will eventually
  argue well, and the metric will agree with it.

The verdict is one of three: **accept** (measurement carried it), **propose**
(measured well, but the class requires a person or the evidence is thin), or
**reject** (measured badly, or a guardrail moved).

## Measuring a config change

`mecha eval --ab-config KEY=VALUE` is the content-sensitive arm: the case set
run twice, differing only in the override, and the difference judged.

```bash
mecha eval --ab-config max_turns=40 eval/cases.jsonl
mecha eval --ab-config compact_at_tokens=8000 --holdout-in 4 -o results/ab.json
```

```text
── config A/B ──
arm A (as configured): 31/40 cases
arm B (max_turns=40): 34/40 cases
  IMPROVED: chaining-deep-traversal
  IMPROVED: audit-multi-file

selection  6+ 2- 19=    holdout  3+ 0- 10=
work       1841 tool calls → 1902

verdict: BETTER — beat the original on the selection slice and held on the holdout
```

**Overrides are a closed set of run options** — `compact_at_tokens`,
`max_turns`, `max_output_tokens`, `effort`. The knobs an automated proposer may
move are exactly the ones a run can be launched with, so both arms are built by
one code path; a second construction site is how two arms silently stop being
comparable. An unknown key is refused, and every override is parsed *before* the
first arm runs, so a typo costs a line of output rather than an hour of
inference.

Neither arm is written as an ordinary scorecard. A scorecard produced under a
candidate override is not comparable to one produced without it, and filing it
as though it were is how an A/B contaminates a series — the same rule
`--ab-rules` already followed.

## Running the whole loop unattended

`mecha harness ruminate` is the nightly verb: diagnose one change from the
corpus, record it as a candidate, measure it by counterfactual replay of recent
sessions, and dispose of it through the gate — in one pass, on a timer.

```bash
mecha harness ruminate --sessions 16 --days 7   # the nightly pass
mecha harness list                              # what is waiting on you
mecha harness show <id>                         # prediction, measurement, evidence
mecha harness overrides                         # the active layer, and where each entry came from
mecha harness revert <id|key>                   # take one back out
```

**A config change that wins on selection, is confirmed on the holdout, and holds
the work guardrail auto-accepts** into a revertible override layer. Everything
else waits for a person: prose and architecture stage unmeasured — prose needs
the content-sensitive arm (`mecha eval --ab-config`), which is a human's spend —
and a `security`-class proposal stages with a standing warning and is **never
measured at all**, because a loop that can argue for widening its own
confinement will eventually argue well and the metric will agree with it.

The override layer is what makes auto-acceptance reversible rather than merely
recorded. `mecha harness revert` takes an entry out and the key returns to
whatever your config says; the candidate record survives as evidence either way.
Exits 0 on "nothing to do" — a skipped night is not a failed night, and the
diagnostician declining to propose is a legitimate answer that is never coerced
into a change.

Everything is recorded, acceptances and rejections alike, so *is this loop
actually helping* is answerable from the store rather than from impression.

## What is deliberately not here

**No model in the gate, and no model applying anything outside the closed set.**
`mecha diagnose` is the one place a model authors a change, and it is safe there
precisely because being wrong costs one measurement — automated failure
attribution is right about which step failed roughly one time in seven. That
property does not hold at the gate, which is why the gate is a pure function.
The set of keys an automated proposer may move is exactly the set a run can be
launched with, so both arms are built by one code path and there is no path from
a measured win to an arbitrary edit.

**No signal that a run went *well*.** Every metric here is phrased as a cost by
deliberate constraint, so this corpus can rank two bad runs and cannot rank two
good ones. That is the gap [goals and appraisal](/docs/features/appraisal)
exists to close, on the other side of the same records.
