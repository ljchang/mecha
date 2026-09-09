# Anchored appraisal pilot — Qwen 3.6 35B

Completed 2026-09-09, 04:44:39–06:02:22 UTC. **Control passed 20/24;
guidance passed 18/24.** There were two improved pairs, four regressions and
18 ties. The native promotion gate rejected guidance. Keep it opt-in.

## Design and conditions

[`eval/appraisal-guidance-v2.toml`](../../eval/appraisal-guidance-v2.toml)
registered eight synthetic tasks × three seeds × two arms, with a 16-turn
ceiling. Both arms ran the configured local `qwen3.6-35b-a3b` model and enabled
step checks. Only goal guidance differed in the materialized arm configurations.
Runtime behavior was frozen at `935d98f5`. All 48 trials completed and were graded;
none were retried, excluded or regraded.

The cases cover dependency-constrained allocation, latest-revision reconciliation,
purpose-specific consent, and release-time scheduling. Each task has an explicit
confirmed goal and a real fixture board record. A synthetic charter and outbox
backlog provide additional goal/sensor context. MCP fixture output retains its
untrusted provenance. Independent grading checks exact JSON artifacts, preserved
inputs and unchanged drafts. Learning and skills are disabled in both arms.

The binary, pinned inputs, served model alias and operator configuration matched
at finish. See [conditions](conditions.json) for hashes and limits. The original
[72-trial baseline](../appraisal-guidance-qwen36-35b-20260909/README.md) is unchanged.

## Results

| Measure | Control | Guidance |
| --- | ---: | ---: |
| Artifact/task passes | 20/24 | 18/24 |
| Mean turns | 12.67 | 12.38 |
| Mean model tool calls | 15.42 | 15.21 |
| Mean output tokens | 8,245.71 | 7,522.50 |
| Mean model-run seconds | 91.64 | 98.50 |
| Executed checks / passed checks | 17 / 8 | 18 / 16 |
| Runs with executed checks | 10/24 | 7/24 |
| Runs with expected confirmed anchor | 24/24 | 24/24 |
| Runs with applied guidance | 0/24 | 24/24 |

All **20 observed omissions of a previously declared check on completion were
restored**, with matching feedback and no `not_declared` outcome. This verifies
the completion-check fix in these traces. It does not make the model's checks an
independent correctness oracle: one wrong guided allocation passed all three
of its checks. No goal-drift writes were observed, so recovery from drift was
not measured. Two guided runs reached the turn limit; all controls completed.

Selection pairs were 1 win / 3 losses / 12 ties; holdout pairs were 1 / 1 / 6.
Repeated seeds are not independent task families. The shared server and running
all controls before guidance confound causal latency/cache comparisons. Cost in
USD and owner-action counts are unknown. Reconciliation cases include stale
rates, but the approved rows with known currencies happen to use USD, limiting
currency-conversion coverage.

## Artifact-contract finding

The privacy briefs relied on workspace `README.txt` to specify `answer.json`.
`guided__privacy-1__s1__r1` instead wrote `share-report.json`; its contents match
the expected answer exactly. Its recorded failure is therefore filename-contract
ambiguous. A different guided run wrote `answer.json` with `send_now: true`, an
actual artifact error; this is not evidence of an external send. The original
grades above are preserved. A post-discovery descriptive slice excluding both
privacy tasks is **15/18 in each arm**, not a replacement gate result.

The separately registered [privacy follow-up](../appraisal-privacy-qwen36-35b-20260909/README.md)
names `answer.json` explicitly and runs every registered seed in both arms.
Its outcomes must not replace or be pooled with these trials. The separate
[learning experiment](../appraisal-learning-v2-qwen36-35b-20260909/README.md)
measures successive-task learning under that explicit contract.

## Evidence and reproduction

[Native export](export.json), [paired summary](summary.json),
[trace observations](trace-observations.json), [resource counts](resources.json)
and [runner log](runner.log) retain the measurements. Raw synthetic sessions
remain in the native experiment store; the trace report records its join method.

```bash
mecha exp export appraisal-guidance-v2-qwen36-35b-20260909 > export.json
python3 scripts/appraisal-report.py export.json eval/fixtures/appraisal-v2/cases.json
python3 scripts/appraisal-traces.py ~/.mecha/experiments/appraisal-guidance-v2-qwen36-35b-20260909 trace-observations.json
```

The regression tests failed before the completion-check fix and pass after it.
Final formatting, warning-free all-feature Clippy, and the required-backend
workspace suite passed: **2,597 tests**, three intentionally ignored. A previously
observed trigger-lock test flaked on one full run, then passed isolated and in a
full rerun; no trigger code changed. Python oracle/report controls also pass.
