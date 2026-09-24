---
title: Goals
sidebar_position: 21.7
description: What a run is for — the goal pointer a plan names, how you confirm it, and how drift from a confirmed goal is measured.
---

# What a run is for

A run can say which of your priorities or board tasks it serves. That pointer
is what lets an [appraisal](/docs/features/appraisal-overview) say what an
outcome was an error *against*. Standing priorities live in
[the charter](/docs/features/charter); this page covers the pointer itself.


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

## Confirming the goal

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

## Explicit goal confirmation for one-shot runs

Use `mecha run --goal task:ID "your task"` to confirm the run's goal. The
reference is recorded before execution and survives resume and compaction.
On `--resume`, omitting `--goal` preserves the saved goal; specifying it replaces
the saved reference. A task reference in prompt text alone is not confirmation.
Experiment manifests can provide the same confirmation with a
`[tasks.confirmed_goals]` table mapping selected case IDs to goal references.

## Measuring goal drift

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
owner question. With `goal_guidance = true`, a plan that differs from the
confirmed goal receives fixed advice to reconcile the mismatch. See
[planning feedback](/docs/features/plan-steps#planning-feedback-and-goal-context) for this opt-in policy.
