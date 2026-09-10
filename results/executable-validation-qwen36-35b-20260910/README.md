# Executable correction pilot — Qwen 3.6 35B

**Keep the 12-turn limit on this evidence.** The proposed 10-turn limit lost two
paired task outcomes with frozen rules and two without them, with no gains in
either cap comparison. On pairs both caps passed, the 10-turn arm used more calls
and turns. This is a synthetic pilot, not a production-wide estimate.

All **96 trials** ran once from 2026-09-10 17:06:40 to 17:24:10 UTC, using runtime
`4b376711` and the input hashes registered before execution. Eight tasks cover seven
patterns (the two linked-packet lengths are variants), with three seeds and four
conditions. Every task executed real builtin file tools in a fresh workspace;
exact output artifacts and preserved source files supplied the independent grade.
No recorded tool outputs were substituted. No model judged task success.

| Frozen rules | Turn cap | Overall and artifact passes | Tool calls | Model turns | Cap hits |
|---|---:|---:|---:|---:|---:|
| Off | 12 | 20/24 | 175 | 138 | 3 |
| Off | 10 | 18/24 | 178 | 134 | 4 |
| On | 12 | 19/24 | 176 | 137 | 4 |
| On | 10 | 17/24 | 172 | 131 | 4 |

Overall and artifact-only counts coincide in this run. A cap hit can still pass
if the required artifact is correct; the limit itself is not a failing grade.
Aggregate costs include unfinished work and must not be read as efficiency gains.
Among both-passing pairs, lowering the cap added **4 calls and 2 turns** with rules
(n=17), and **9 calls and 3 turns** without rules (n=18). Timing is descriptive:
the native runner uses fixed arm order on a shared local service.

The cap regressions were `linked-packet-8`, seeds 1 and 2, under both rule settings.
The 12-turn arms completed the artifact; the 10-turn arms reached the limit without
correcting it. Both lengths permit listing and batching reads; no fixed trace is
required by their oracle.

Rules produced **one paired improvement and two regressions at each cap**. They
hurt `latest-evidence` seed 2, helped `linked-packet-11` seed 1 and hurt its seed 3.
This mixed result is not a rule-level attribution or a basis for bulk retirement.
The active rules were frozen from the operator's store before execution; no rule
was learned, changed or retired during the pilot.

## Conditions and limits

The recorded tool surface was exactly `fs_edit`, `fs_list`, `fs_read`, `fs_write`
and `todo`. All 96 configuration audits matched their model, seed, cap and rule
condition. The rules-off block carried the empty-block hash; rules-on carried
`39a27bbb5ef07c9e`. Execution inputs and server alias/slot/context readings matched
at start and finish. The post-run [report correction](report-correction.json) fixed
an empty-hash interpretation error; [the original audit](initial-scorecard.json)
and [its code](frozen-runner.py) are retained. No model trials were rerun.

These tasks start from registered incorrect artifacts; they do not reconstruct
historical conversations or external-service state. The rules comparison tests
frozen exposure, not the learning process. Seed repeats are not independent task
families, and the native gate's selection/holdout split does not establish transfer
to unseen work. The cap is a model-turn limit, not an individual tool-call limit.

The report references staged proposal `hc-20260910T034920-40f9` (`max_turns=10`).
No production config, candidate status, service or operator learning store was
changed. Representative task families and live-service fixtures remain necessary
before generalizing these results or automating artifact-based promotion.

## Evidence and reproduction

[Scorecard and per-task results](scorecard.json), [native export and checks](export.json),
[registered manifest](manifest.toml), [conditions](conditions.json),
[finish audit](finish.json), [trial plan](plan.log), [run log](run.log).
The full private archive, including original/final workspace files, transcripts and
frozen operator rules/config, is at `/tmp/mecha-executable-validation-v1-20260910`.
It is deliberately not copied into this repository. Reproducing the exact rule
condition requires that snapshot; using a different rule set is a new condition.

```bash
python3 scripts/executable-validation.py --out /tmp/new-executable-pilot
python3 scripts/executable-validation.py --out /tmp/new-executable-pilot --report-only
```

The checked-in fixture builder and source define the synthetic task world; the
runner accepts `--binary`, `--config`, `--rules` and a matching `--candidate` file.
It refuses to overwrite an existing run. The report-only mode executes no trials.
Build, formatting, warning-free Clippy and required-backend workspace tests passed:
**2,626 tests passed, zero failed, three intentionally ignored**, including the new
native manifest/oracle test. The Python oracle/report suite passed four tests.
