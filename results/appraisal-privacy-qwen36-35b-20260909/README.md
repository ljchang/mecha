# Explicit-artifact privacy follow-up — Qwen 3.6 35B

Completed 2026-09-09, 06:02:23–06:14:09 UTC. **Control passed 6/6;
guidance passed 5/6:** zero improved pairs, one regression, five ties.
The sole failure, `guided__privacy-1__s2__r1`, was `artifact excluded_count`.
All trials produced the required artifact; all input/draft-preservation checks
passed. There were no retries, exclusions or regrading.

## Why this is a separate experiment

The [harder pilot](../appraisal-guidance-v2-qwen36-35b-20260909/README.md)
exposed a filename-contract ambiguity. This follow-up explicitly requests
`answer.json` in every prompt using
[`appraisal_artifact_source.py`](../../eval/fixtures/appraisal_artifact_source.py).
It retains the privacy inputs and independent expected contents, and runs both
privacy tasks × all three seeds × guidance off/on. The manifest was registered
before these trials. These results do not replace or pool with the original grades.

Both arms use local `qwen3.6-35b-a3b`, a 16-turn ceiling, confirmed goals,
real fixture board records and enabled step checks. Learning is disabled.
Normalized final configurations differ only in goal guidance. Pinned binary and
inputs, model alias and operator configuration matched at finish.

## Measurements

| Measure | Control | Guidance |
| --- | ---: | ---: |
| Task passes | 6/6 | 5/6 |
| Mean turns | 11.33 | 11.83 |
| Mean model tool calls | 13.67 | 14.67 |
| Mean model-run seconds | 61.46 | 55.56 |
| Executed / passed checks | 5 / 5 | 5 / 5 |
| Runs with expected anchor | 6/6 | 6/6 |
| Runs with applied guidance | 0/6 | 6/6 |

All four observed completion-time check omissions were restored, with matching
feedback. All twelve runs completed normally. Usage is fully recorded; USD cost
and owner-action counts remain unknown.

The native gate returned `Propose`, meaning insufficient evidence for automatic
promotion: only two selection pairs, below its floor of eight. Selection had two
ties; holdout had one loss and three ties. This small follow-up establishes no
benefit from guidance. Repeated seeds, fixed arm order and a shared model server
limit inference. Artifact correctness is not a measure of real external sending
or owner-policy outcomes. Guidance remains opt-in.

## Evidence

[Manifest](../../eval/appraisal-privacy.toml), [conditions](conditions.json),
[native export](export.json), [paired summary](summary.json),
[trace observations](trace-observations.json), [resources](resources.json)
and [runner log](runner.log).

```bash
mecha exp export appraisal-privacy-qwen36-35b-20260909 > export.json
python3 scripts/appraisal-report.py export.json eval/fixtures/appraisal-v2/cases.json
python3 scripts/appraisal-traces.py ~/.mecha/experiments/appraisal-privacy-qwen36-35b-20260909 trace-observations.json
```
