---
title: Anticipatory appraisal
sidebar_position: 6
description: Recording a concern before an action — owner-supplied evidence, per-draft predictions and release checks, and the owner verdicts recorded after delivery.
---

# Anticipatory appraisal and outcome evidence

[Appraisal](/docs/features/appraisal) usually reads an outcome after
it happened. Anticipatory appraisal records a concern **before** the action,
and the outcome evidence below links what later happened back to that
prediction.


Anticipatory appraisal records a concern **before an action**, separately from
what later happened. The current slice covers confirmed-goal planning and
outbox messages with inline prose. It makes no additional model calls.

| Assessment | Evidence it uses |
|---|---|
| Anticipated guilt | A recorded commitment, its beneficiary and possible adverse consequence, plus missing verification or a threatened budget. A goal reference alone does not establish a commitment. |
| Anticipated embarrassment | An outgoing message with a relevant expectation or check that has not been verified. It does not mean an error occurred. |
| Anticipated regret | A relevant check is available and its recorded cost fits the available time: checking first is a feasible alternative to proceeding unchecked. |
| Anticipated disappointment | A recorded expected outcome is threatened by failed verification or insufficient budget. |
| Anxiety/concern | Failed verification or work/check cost exceeding the remaining budget. |
| Curiosity/interest | A decision-relevant unknown has an available, affordable check. |

These are deterministic assessments, not calibrated probabilities or claims of
experienced emotion. Unknown cost stays unknown. Checking is not always the
best action: when its cost exceeds the available time, the advice is to review
the commitment and choose a fallback or smaller scope.

## Supply evidence before a run

Write an owner-authored JSON file, for example `meeting-evidence.json`:

```json
{
  "goal": "task:meeting",
  "commitment": {
    "beneficiary": "meeting attendees",
    "expectation": "Send the confirmed meeting time",
    "consequence": "An incorrect time could cause someone to miss the meeting"
  },
  "expected_outcome": "An accurate invitation",
  "verification": "unknown",
  "check_available": true,
  "check_cost_secs": 10,
  "time_available_secs": 600
}
```

Use the actual task reference and facts for your run. These fields record your
assessment; the harness does not verify an estimate merely because it is in the
file. Third-party assertions are not imported as commitments.

The `commitment` shape above keeps working. mecha records it as the same
commitment record `mecha workflow commit` writes: the beneficiary becomes the
`party`, the file's `goal` becomes its `source`, and it has **no deadline** —
that shape has no date to carry. You may also write the record shape directly
(`party`, `source`, `expectation`, `consequence`, and optionally `due_at` and
`follow_up_at` as timestamps with a timezone); its `source` must then be the
file's `goal`, and any dates you write are kept exactly as written, with the
follow-up no later than the deadline. mecha never supplies a date itself.
Unknown keys are refused in either shape.
Drafts assessed before this change keep their original commitment exactly as
recorded, and still release normally.

```bash
mecha run --goal task:meeting --appraisal-evidence meeting-evidence.json \
  "Check the meeting time and draft the invitation"
```

The file's goal must match `--goal`. Evidence applies to this invocation;
resuming requires explicitly supplying it again. Plan updates record the
assessment and its supporting evidence locally. Enable `[agent] goal_guidance =
true` for fixed planning advice; otherwise it is observational. Goal alignment
and ordered charter findings retain precedence. A changed or unnamed plan does
not inherit an unrelated commitment. Available time decreases during the run.

Raw commitment text, numeric readings and verification evidence remain local
metadata. Provider requests carry only the fixed guidance. A check of prior
context does not certify a newly authored message: the draft inherits the
commitment and remaining budget, but its verification starts unknown.

## Assess a specific draft before release

New inline message drafts keep an initial prediction automatically. Inspect the
prediction IDs, sources, evidence, recommended response and resolution:

```bash
mecha outbox anticipate DRAFT_ID
```

Attach or revise owner evidence against the draft's exact current arguments:

```bash
mecha outbox anticipate DRAFT_ID --file meeting-evidence.json --guide
```

`--guide` opts this item into a release check: its latest assessment must still
match the arguments and support proceeding before the shared delivery path can
execute it. It does not run the proposed check for you. Perform the check, then
record `"verification": "passed"` and a nonempty `"verification_evidence"`
describing what was checked, and reassess. A pass establishes only what that
check tested. Failed checks, missing evidence, expired commitment time and
budget shortfalls can still require clarification or replanning.

```bash
mecha outbox anticipate DRAFT_ID --file checked-evidence.json
```

Omitting a mode flag preserves an existing guidance requirement. Use `--observe`
with `--file` to explicitly select observation mode. Editing and rejecting remain
available when guidance prevents release; editing invalidates an assessment of
different arguments. Earlier predictions remain in the history. All release
surfaces must use this implementation to enforce the new guidance requirement;
older binaries do not interpret the new fields.

This version preserves unsupported prediction and outcome records as raw JSON.
Drafts remain visible, editable and rejectable. Unsupported evidence blocks
release and is reported as ungraded; it never becomes a positive outcome.
A newer prediction can be replaced by an explicit reassessment with `--guide`
or `--observe`; unknown outcome history requires a compatible binary.
If the current appraisal cannot be recomputed, `outbox show` displays a warning
alongside the draft; the delivery check still refuses the action.

## Record what happened

After confirmed delivery, record an owner verdict against the prediction that
was used for that action. For example, `outcome.json`:

```json
{
  "prediction_id": "PREDICTION_ID_FROM_THE_READOUT",
  "verdict": "error_exposed",
  "evidence": "I checked the delivered message: it stated 10:00, while the confirmed meeting was 11:00.",
  "attributable_to_mecha": true
}
```

```bash
mecha outbox outcome DRAFT_ID --file outcome.json
mecha sessions appraise --days 30
```

The supported verdicts are `error_exposed`, `harm`, `expectation_missed`,
`no_issue` and `withdrawn`. `harm` additionally requires a recorded commitment on
the prediction. Attributing an outcome to mecha currently requires an unchanged,
model-authored message; an owner's rewritten message is not attributed to mecha.
Uncertain delivery must be reconciled before feedback can establish an outcome:
`mecha outbox outcome` requires a draft that is `sent` with no unknown delivery
attempt. When a send's result was unknown, check the destination and record what
you found, which never resends:

```bash
mecha outbox reconcile DRAFT_ID --outcome delivered --evidence "Appears in Sent at 14:02"
```

The web outbox offers the same reconcile action. Confirmed delivery is therefore
the precondition for the retrospective `embarrassment` and `guilt` labels below.

A linked exposed error can produce retrospective `embarrassment`; an attributable
impact can produce `guilt`. An expectation miss is a negative owner verdict; it
does not establish that no better alternative existed or replace the existing
counterfactual distinction between regret and disappointment. These owner outcomes
do not themselves stage automatic task follow-ups or author learned rules.

Only one outcome per draft is active. To revise it, include `supersedes` with the
previous outcome ID; withdrawal also requires that ID. Both records remain.
The active negative verdict replaces the initial drafting verdict for that
incident, avoiding duplicate rewards or penalties.

A prediction can remain `pending`, become `changed`, `reassessed`, or `abandoned`,
have `delivery_unknown`, await feedback, or become `observed`. Changing a message
or checking before proceeding does not establish that the original forecast was
wrong. Silence after delivery is not evidence of success. Retrospective guilt
and embarrassment can occur even when the earlier check passed.

## How well the predictions held up

`mecha sessions appraise` scores every prediction in the outbox once an
outcome resolves it. The scores are grouped by the response the assessment
chose (`proceed`, `verify`, `clarify`, `replan`) and by each concern it named.
A scored prediction either had its concern materialise (you recorded an
exposed error, a harm or a missed expectation) or went out clean.

- **Clean needs a confirmed delivery.** Either the send was acknowledged, or
  you reconciled it with `mecha outbox reconcile`. A "no issue" on a draft
  whose delivery was never confirmed waits as `delivery_unconfirmed`.
- **Every other prediction is shown as coverage**, never as a score: not
  sent, awaiting your outcome, delivery unknown, changed, reassessed,
  abandoned or unsupported.
- **A rate appears only where there is something to rate.** Until you record
  outcomes, the readout shows coverage alone, and `materialized_rate` is
  `null` in `--json`.

```text
  anticipation's predictions: 3 scored of 14 (proceed 1/5 scored, concern materialised 0% · verify 2/2 scored, concern materialised 50% · clarify 0/7 scored, no rate) · not yet a point: 1 awaiting the owner's outcome, 1 not sent, …
```

## Where this is available

Supplying evidence (`mecha run --appraisal-evidence`), assessing a draft
(`mecha outbox anticipate`) and recording an outcome (`mecha outbox outcome`) are
command-line only today. The TUI, the web outbox and Slack show and release
drafts but have no control for predictions or outcomes; an outcome you want
counted has to be recorded from a terminal.

## Current measurement limits

The deterministic store, planning and CLI tests cover the complete record flow;
they do not establish improved real-world outcomes. The current regret assessment
compares an available check with proceeding unchecked, not arbitrary alternative
plans. Automatic semantic harm detection, pattern-based shame, excitement,
mutable file-bundle verification and automatic learning from these outcomes are
outside this slice.

Run configuration records retain owner-bound evidence. Counterfactual probes,
artifact-task validation, whole-session harness probes and replay refuse unsupported reproductions of those
runs rather than grade a run after silently dropping its evidence. Ordinary
appraisal and prediction readouts remain available. Broader measured guidance
comparisons require extending that reproduction path first.
