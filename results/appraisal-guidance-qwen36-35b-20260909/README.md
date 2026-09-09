# Qwen 3.6 35B appraisal guidance pilot — 2026-09-09

Both arms passed **36/36 trials**. The pilot found no task-success improvement from guidance; the existing comparison gate rejected promotion with 24 selection ties and 12 holdout ties. Guidance remains opt-in. This is a ceiling result on twelve synthetic tasks, not evidence that guidance can never help.

Ran from 03:20:32 to 04:06:01 UTC against source `f3fe4df8c1e9c9bd39d3b7360162357c26927232`, using the local `qwen3.6-35b-a3b` model (`Qwen3.6-35B-A3B-UD-Q4_K_M.gguf`). Each of twelve tasks ran with seeds 1, 2 and 3 in control and guided arms, at a 24-turn ceiling. The first trial checked live setup; the remaining 71 resumed the same registered design. All trials finished and were graded; none were retried, excluded or regraded.

## Paired outcomes and resource use

Each mean below is over the same 36 matched task/seed/repetition pairs.

| Measure | Guidance off | Guidance on |
|---|---:|---:|
| Artifact/task acceptance | 36/36 | 36/36 |
| Mean turns | 8.83 | 8.97 |
| Mean model tool calls | 12.86 | 13.03 |
| Mean duration (seconds) | 34.90 | 39.48 |
| Mean output tokens | 2023.92 | 1929.39 |
| Mean total input tokens across requests, including cache | 70307.00 | 69230.64 |
| Runs with executed declared checks | 13/36 | 12/36 |
| Declared checks executed / passed | 25 / 17 | 21 / 14 |
| Reopened steps | 0 | 0 |
| Null steps / measurable completions | 6 / 102 | 7 / 102 |

Guidance added five turns and six model tool calls across the 36 matched trials; output tokens fell by 3,403. These small descriptive differences do not establish a general resource benefit. Usage was complete in all 72 records. Dollar cost and owner actions are **unmeasured**, not zero. Native harness-check failures are distinct from the independent artifact verdict: both arms still passed every task.

Wall-clock means are not a causal estimate: the driver ran all control trials before all guided trials, and the shared server showed concurrent activity during the pilot. Request caching also depends on run order. The twelve tasks, not 36 seeds, are the distinct task cases; the driver's native selection/holdout split is over trial pairs rather than held-out task families.

## What the live traces established

- Live rendered configurations differed only in `agent.goal_guidance`, after normalizing trial-home paths and per-trial seeds. Both arms used the same model, sandbox and turn ceiling, with step checks enabled. The fixture servers replaced live MCP servers; learned rules and historical examples were disabled in both arms.
- Guidance was recorded as applied in **36/36 guided runs and 0/36 control runs**. All six pressure trials recorded the expected charter discrepancy and selected review first; all six quiet trials selected polish. All draft-preservation checks passed. These facts verify exposure and fixture behavior, not added decision quality over the control.
- **Checks can disappear on the completing update.** There were 38 observed omissions across 26 control runs and 33 across 25 guided runs. Every one matched a persisted `not_declared` observation. `Tracked::advance` in `mecha-core/src/tool/todo.rs` restores an omitted check only when it was already frozen; the first completing update can omit an open step's check without freezing or executing it. Example: `control__budget__s1__r1`, completion call `ouuRFHpYvlbkZezrnjgq0Wlklp3G9Gvd`. The code was kept fixed throughout the pilot.
- **No run had a confirmed goal anchor.** The tasks supplied goal references in their briefs, but did not invoke the confirmation flow or seed corresponding board tasks. Goal-drift counters therefore had no anchored-plan exposure; zero drift cannot establish effectiveness.
- Learning stages, learned context, semantic owner corrections across turns and real owner-policy outcomes were not exercised.

## Next work

First close the completion-time omission path: declare a failing check while a step is open, complete it without the field, and require the original check to remain frozen and execute through the normal guards. Add a regression that fails on this recorded behavior. Then design harder tasks with confirmed anchors and real task records, and separately test learning across runs. Register a new experiment name for changed code or fixtures; preserve this pilot as the baseline.

## Artifacts and reproduction

- [Registered-design export](export.json): all 72 graded trials, checks, native stats, manifest and gate judgement.
- [Paired summary](summary.json): metric-specific denominators and missing observations.
- [Conditions](conditions.json): source and binary identity, fixture hashes, model settings, timestamps and server observations. Model identity, binary, fixture files and operator configuration matched their initial values at the finish checks.
- [Resource totals](resources.json): normalized usage counters; total input is uncached input plus both cache tiers, summed across requests.
- [Trace observations](trace-observations.json) and [analysis script](analyze-traces.py): accepted todo inputs joined to persisted planning metadata; exact call IDs retained for the omission finding. Compacted/rewritten transcripts would be marked unknown; none occurred.
- [Runner log](runner.log): sequential execution and verdicts.

Full transcripts and trial workspaces remain in the local experiment store at `~/.mecha/experiments/appraisal-guidance-qwen36-35b-20260909/`. From the repository root, rebuild the paired report with:

```bash
python3 scripts/appraisal-report.py results/appraisal-guidance-qwen36-35b-20260909/export.json
python3 results/appraisal-guidance-qwen36-35b-20260909/analyze-traces.py
```

The second command needs the local transcripts. The registered manifest is `eval/appraisal-guidance.toml`; a fresh model run needs a new experiment name because registration is immutable. No installation, service restart, default-guidance change or PR was made.
