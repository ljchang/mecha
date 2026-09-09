# Sequential appraisal learning pilot — Qwen 3.6 35B

Completed 2026-09-09, 06:14:09–06:30:41 UTC. **Both arms passed 6/6 tasks,
including 3/3 on the transfer slice.** The treatment produced two clean,
goal-linked reflections, but **no learned rules and no rule exposure**.
This verifies reflection capture; it does not demonstrate improved learning
or the effect of learned rules on later work.

## Registered design

[`eval/appraisal-learning-v2.toml`](../../eval/appraisal-learning-v2.toml)
defines one ordered, six-task lifetime per arm, seed 1. Both use local
`qwen3.6-35b-a3b`, 16 turns, confirmed task goals, goal guidance, step checks
and learned-rule loading. MCP, skills, hooks, messages and escalation are off
in both. Their materialized task configurations are identical. Both start with
empty learning stores; only the treatment runs learning stages.

The fixed sequence is `reconcile-1`, `privacy-1`, `schedule-1`, then
`reconcile-2`, `privacy-2`, `schedule-2`. Positions 4–6 were designated the
transfer slice before execution. Every prompt explicitly names `answer.json`.
The earlier [learning design](../appraisal-learning-qwen36-35b-20260909/README.md)
was superseded before any model run because its artifact contract was ambiguous.

The bootstrap home was `/tmp/mecha-appraisal-learning-v2-20260909`, seeded only
with the native runner's scrubbed configuration from the preceding pilot.
No owner rules or transcripts were copied. Local files are maintainer-authored
synthetic fixtures under the existing local-file provenance policy; this does
not establish safe learning from real third-party content. The ordinary MCP
provenance guard was not weakened to make the experiment learn.

## What actually happened

| Measure | Control | Learning stages enabled |
| --- | ---: | ---: |
| Initial task passes | 3/3 | 3/3 |
| Transfer task passes | 3/3 | 3/3 |
| Completed scheduled stages | 0 | 12 |
| Reflections created | 0 | 2 |
| Learned rules created | 0 | 0 |
| Runs with learned rules | 0/6 | 0/6 |
| Validation records | 0 | 0 |

All twelve trials completed normally and were graded. There were no retries,
exclusions or unreadable records. Both arms recorded the expected anchors and
applied guidance on every run. All four observed completion-time check omissions
were restored with matching feedback.

The treatment ran six reflection stages and two each of validation, learning
and retirement. The ledger retains both running and terminal entries; the latest
status is `done` for all twelve stages. Reflections came from harness-recorded
check mismatches on `privacy-1` and `privacy-2`, with clean provenance, full
evidence and the corresponding task goal. At the first consolidation boundary,
one reflection was available; at the last, two. Both were below the unchanged
minimum of three. Neither was processed into a rule. Validation had no rules
to evaluate; probation status is inapplicable because none were created.

The native gate returned `Propose` for insufficient selection pairs (two,
below its floor of eight), with all six pairs tied. The explicit transfer slice
is separate from that native random holdout. This short sequence reached an
accuracy ceiling and did not exercise rule consolidation or transfer. A future
measurement needs more naturally occurring, eligible mismatch evidence before
the transfer tasks, while preserving the minimum, scope and provenance gates.
A mismatch-specific counterfactual grader remains unimplemented. The learner
never sees the independent oracle gold or artifact verdicts in this design.

## Evidence and reproduction

[Conditions and hashes](conditions.json), [native export](export.json),
[summary](summary.json), [trace observations](trace-observations.json),
[learning evidence inventory](learning-evidence.json),
[raw synthetic reflections](learning-evidence/learning/reflections.jsonl),
[stage logs](stage-logs/learning__s1__r1), and [task resources](resources.json).
Task resource counts exclude model calls made by learning stages, so they are
not an end-to-end learning cost comparison. Fixed arm order and shared server
activity further limit latency inference.

The served model alias, binary, pinned inputs and operator config matched at
finish. No learning state was deployed or copied back to the operator's home.

From the repository root, use a fresh bootstrap home with a local-provider
configuration and empty learning/skills/session stores:

```bash
MECHA_HOME=/path/to/fresh-bootstrap mecha exp new eval/appraisal-learning-v2.toml
MECHA_HOME=/path/to/fresh-bootstrap mecha exp run appraisal-learning-v2-qwen36-35b-20260909
MECHA_HOME=/path/to/fresh-bootstrap mecha exp export appraisal-learning-v2-qwen36-35b-20260909 > export.json
python3 scripts/appraisal-traces.py /path/to/fresh-bootstrap/experiments/appraisal-learning-v2-qwen36-35b-20260909 trace-observations.json
python3 scripts/appraisal-learning-report.py export.json trace-observations.json
```
