# Grounded attribution pilot — Qwen 3.6 35B

**The corrected pilot completed all 72 trials and 54 learning stages, but did not
reach learned-rule exposure or demonstrate a learning benefit.** Both arms passed
30/36 overall and 15/18 unseen transfer tasks. All 36 paired task outcomes tied,
and the native gate rejected promotion. No rule was created, loaded or retired.

Trials ran 2026-09-09, 15:29:20–16:03:08 UTC.
Runtime: `c7071ad3`, server-confirmed `qwen3.6-35b-a3b`, three seeds with fresh,
separate control/learning homes. [METHOD.md](METHOD.md) describes the registered
six-training/six-transfer design, sixteen-turn budget, five-tool surface, fixed
arm order, provenance boundaries and limitations. [Conditions](conditions.json)
and the [finish audit](finish-audit.json) confirm unchanged runtime/input hashes
and the model alias. No runtime or fixture changed during this valid measurement.

**Historical runtime boundary:** this run predates the check-provenance/replay
review fixes in PR #220 and the anticipatory appraisal implementation in PR #221.
It does not validate current main, live Anthropic compatibility, or anticipatory
guidance. All raw records and native grades retain the measured runtime's behavior.

## Outcomes

| Measure | Control | Learning |
| --- | ---: | ---: |
| Training overall passes | 15/18 | 15/18 |
| Transfer overall passes | 15/18 | 15/18 |
| All overall passes | 30/36 | 30/36 |
| Training artifact passes | 15/18 | 15/18 |
| Transfer artifact passes | 16/18 | 16/18 |
| All artifact passes | 31/36 | 31/36 |
| Runs with learned rules | 0/36 | 0/36 |

Each seed's outcomes matched between arms:

| Seed | Training overall | Transfer overall | All artifact passes |
| --- | ---: | ---: | ---: |
| 1 | 4/6 | 6/6 | 10/12 |
| 2 | 5/6 | 5/6 | 10/12 |
| 3 | 6/6 | 4/6 | 11/12 |

All six training artifact failures across the two arms were `review_priority`
errors; selection and signed totals passed their registered criteria. Failures
included treating the available `outbox_waiting` field as absent because the
model expected another field name. On seed 2's missing-reading transfer task,
the model instead substituted the selected-record count for the missing queue
reading. The exact artifact checks preserve these as failures.

On seed 3's missing-reading task, both artifacts were correct but the native
`no invented tools` check failed. The transcript shows a more precise mechanism:
the model declared `check: "file reads"`, then omitted it on completion. The
harness retained the check and generated a `shell` call, which the five-tool
registry could not execute. That call is explicitly marked `harness=true`;
it was not a direct model-invented tool call. Native grades remain unchanged,
and artifact correctness is reported separately.

## What learning actually did

The three learning lifetimes collected **2, 1 and 0 training reflections**.
They were clean, goal-linked criterion mismatches; the priority reflections
also retained the owner-supplied charter association. Passing tasks produced no
follow-up reflections. The [dry-run mining audit](mining-audit.log) independently
checks one failed and one passing real transcript without calling a model.

Every lifetime remained below the unchanged minimum of three reflections.
Consequently, all consolidation checkpoints skipped; validation had no rules to
probe, and there are no candidate proposals or paired rule-validation receipts.
All 72 transcript configurations confirm zero rule exposure. A completed stage
is not evidence that learning was applied.

Seed 3 later produced one ordinary check-change mismatch on transfer, from the
omitted declaration described above. This was deduplicated despite repeated
feedback records. It contains no artifact criterion or evaluation answer, remains
below the evidence minimum, and did not produce a rule. Final reflection counts
are **2, 1 and 1**, all clean/full-evidence mismatches and all unprocessed. There
are **zero artifact-derived criterion observations on transfer**.

The observed context reaches only the quarantined post-run reflector. This
experiment uses pinned synthetic count snapshots and an owner-supplied charter
pointer; it does not exercise live charter/sensor stores, rates, durations,
staleness handling or actual owner-policy outcomes. Equal scores with zero rule
exposure cannot establish whether learned guidance helps or hurts.

## Attribution and validation limits

The call-estimate audit found 54 paired estimates in control and 53 in learning;
98 and 100 other step observations lacked an estimate or a measurable span.
Neither arm crossed the implemented forecast-miss threshold. The conservative
cost-only learning filter is regression-tested, but this flat-file pilot does
not itself exercise large overruns or establish improved calibration. The native
312 versus 304 task-call totals exclude learning-stage model calls and are not
evidence of a learned efficiency gain.

The [initial pilot](../appraisal-attribution-qwen36-35b-20260909/README.md) was
stopped after 40 completed trials when generic harness diagnostics were being
mined as owner follow-ups. Its results are excluded here. The fix honors the
harness marker and recognizes historical diagnostic text; its regression test
failed before the fix. The mismatch prompt also distinguishes a false context
predicate from a failed artifact verdict. The replacement reused no trial or
learning state from the invalid run.

At runtime `c7071ad3`, build, formatting and warning-free Clippy passed. Required-backend workspace
tests passed **2,616**, with zero failures and three intentionally ignored tests.
The separate [retirement drills](retirement-drills/README.md) must be read with
their outcomes: an earlier run passed, but the rerun on the corrected runtime
**failed its strict assertion** because the bad rule elicited no measured
regression. Neither rule retired in that rerun. It was not retried for a pass,
and it does not demonstrate live retirement on the final runtime.

Guidance stays opt-in; no model defaults, installed binaries, services or operator
learning state were changed. The next useful measurement needs enough trusted,
predeclared training failures per independent learner to cross the existing gate,
then actual rule exposure on unseen tasks. Lowering the gate or pooling isolated
seeds after seeing these outcomes would answer a different question.

## Evidence and reproduction

[Native export](export.json), [native judgement](judge.log),
[sequence summary](summary.json), [artifact scores](artifact-scores.json),
[artifact checks by trial](artifact-trials.json), [rule exposure](trace-observations.json),
[configuration audit](config-audit.json), [calibration observations](calibration.json),
[step feedback](step-feedback.json), [learning inventory](learning-evidence.json),
[final rule roster](final-rules.json), [transcripts](transcripts),
[learning records](learning-evidence), [stage logs](stage-logs),
[task resources](resources.json), and [verification logs](verification).
JSONL records are archived as JSON arrays; mined-session IDs remain plain text.
The archive helper records this run's extraction and expects its original
temporary logs and fingerprint paths. To repeat the registered design, build and
use `mecha` from source commit `c7071ad3`, use that checkout's fixtures/scripts,
and match the provider settings in [Conditions](conditions.json). The commands
below then run a fresh experiment; model outcomes are not guaranteed to repeat.
Running them on current main measures a different implementation.

```bash
MECHA_HOME=/path/to/fresh-bootstrap mecha exp new eval/appraisal-attribution-v2.toml
MECHA_HOME=/path/to/fresh-bootstrap mecha exp run appraisal-attribution-v2-qwen36-35b-20260909
MECHA_HOME=/path/to/fresh-bootstrap mecha exp judge appraisal-attribution-v2-qwen36-35b-20260909
MECHA_HOME=/path/to/fresh-bootstrap mecha exp export appraisal-attribution-v2-qwen36-35b-20260909 > export.json
python3 scripts/appraisal-traces.py /path/to/fresh-bootstrap/experiments/appraisal-attribution-v2-qwen36-35b-20260909 trace-observations.json
python3 scripts/appraisal-learning-report.py export.json trace-observations.json
```
