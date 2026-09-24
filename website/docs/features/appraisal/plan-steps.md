---
title: Plan steps and checks
sidebar_position: 5
description: How a ticked-off plan step is checked against what it actually did, declared step checks, opt-in planning guidance, and validating planning lessons against owner fixtures.
---

# Plan steps and checks

Most appraisal reads work after the run. This page covers the part that runs
**inside** it: each plan step is checked when the model marks it done, and
what the checks find can feed planning guidance and learned rules. See
[How appraisal works](/docs/features/appraisal) for the whole picture.

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
tuned constant. Five findings:

| Finding | The span says |
|---|---|
| **landed** | the common case, and it renders *nothing* — a line per honest step would be bulk carried in the transcript for the rest of the run in exchange for confirming what the model already believes |
| **the null step** | zero tool calls: the box was ticked and nothing was attempted |
| **ended on failure** | the last thing tried failed, and nothing after it succeeded — a failure *among* successes is recovery, and recovery is the model working |
| **ended on refusal** | the last thing tried was refused: the step was **blocked**, not botched — telling the model otherwise would send it to fix code that is working |
| **check failed** | a check the step itself declared ran and did not pass — read before the others, because the last call may have succeeded while the claim did not |

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

When a completion update omits a check declared while the step was open, the
harness restores and freezes that check and executes it through the usual guards.
A check explicitly withdrawn while the step is still open stays withdrawn.

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
when requested by the agent. Failed checks, changes to frozen checks and
verified task-criterion failures can supply bounded mismatch reflections to
`mecha reflect`. Estimate overruns remain observations; they do not establish a
behavioral lesson. Unknown or tainted mismatch evidence is excluded.

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
