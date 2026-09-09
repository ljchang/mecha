# Independent mismatch validation — Qwen 3.6 35B

Completed 2026-09-09, 13:50:41–14:26:51 UTC, on runtime commit `edb554a7`.
**One rule was learned and loaded in all six transfer runs, but performance did
not improve:** control passed 10/12 overall and 6/6 transfer, versus learning
9/12 overall and 5/6 transfer. The native gate rejected promotion. All 24 tasks
and 18 scheduled learning stages completed. Guidance remains opt-in.

The [manifest](../../eval/appraisal-mismatch.toml) defines twelve ordered tasks
per arm: three revision tasks and three consent tasks for initial learning,
then three new tasks of each kind for transfer. Both arms use the server-confirmed
`qwen3.6-35b-a3b` alias, a 24-turn budget, confirmed task goals, plan checks and
learned-rule loading. Only the learning arm runs reflection after each task and
validation, consolidation and retirement after positions 6 and 12. Goal guidance,
MCP, hooks, skills, messaging, fallback, escalation and outbox routing are off.
The supported tools are `fs_read`, `fs_list`, `fs_write`, `fs_edit` and `todo`.

Both learning stores start empty. The operator configuration was copied through
the native runner's scrubbed configuration into a fresh bootstrap home; only that
isolated home grants unattended fixture writes. No owner rules, transcripts or
skills were copied. Inputs, gold, prompts, runtime binary and operator configuration
were fingerprinted before execution in [conditions.json](conditions.json).
No experiment fixture or runtime behavior is changed during measurement.

Each task follows twelve linked records, resolving revision or purpose-specific
consent rules. Queue context and owner priorities are synthetic fixture data,
not live sensors or the operator's charter. The independent oracle checks exact
JSON and preserved inputs. Native task grading additionally checks tool use;
an otherwise correct artifact can therefore fail the overall task score.
The learner receives harness-recorded planning mismatches, not oracle answers.
All fixture files, including the vendor memo, are maintainer-authored local data;
this does not demonstrate safe learning from real untrusted external content.

The existing minimum remains three reflections. The native learning stage passes
`--holdout 0.25 --auto`; its deterministic implementation reserves every fourth
reflection by ID, so a three-reflection pool reserves zero. Candidate validation
on training reflections and the later task-transfer slice are separate measures.
Whole-task artifact repeats do not reconstruct a mid-step filesystem or validate
the accuracy of a tool-call forecast. Their ledger rows claim no regional step
coverage. Unsupported evidence remains skipped; refused writes remain inconclusive.

The [controlled drill](controlled-artifact-drill/README.md) is separate synthetic
evidence. It detected an aggregate rule regression, but both singleton subsets
passed and attribution abstained. Its strict attribution/retirement assertion
failed; neither rule was retired. The existing trace-based retirement drill passed.
Neither drill is counted as naturally acquired learning or included in pilot scores.

## Task outcomes and exposure

| Measure | Control | Learning |
| --- | ---: | ---: |
| Initial overall passes | 4/6 | 4/6 |
| Transfer overall passes | 6/6 | 5/6 |
| All overall passes | 10/12 | 9/12 |
| Initial artifact passes | 5/6 | 5/6 |
| Transfer artifact passes | 6/6 | 5/6 |
| All artifact passes | 11/12 | 10/12 |
| Initial runs with learned rules | 0/6 | 0/6 |
| Transfer runs with learned rules | 0/6 | 6/6 |

Both arms made the same incorrect `review_first=true` decision on revision-2,
where the queue context required false. Both produced correct consent-3 artifacts
but called nonexistent tools twice. The learning arm also chose the wrong
`review_first` value on revision-6, whose control counterpart passed. Every
registered task finished and was graded; no task was retried or excluded.

The first six tasks produced three clean, goal-linked mismatch reflections.
All three shared the `todo` focus and met the unchanged minimum. The first
consolidation applied rule `r-20260909-c3029194` after three paired artifact tests:
zero improvements, zero regressions, three unchanged (both-pass) results. The
rule asks the agent to halt and verify when actual calls substantially exceed
its estimate, and to stop unnecessary work. It is scoped to runs exposing
`todo`. Every transfer transcript records that rule ID; its effect was therefore
exposed, rather than inferred from an enabled learning switch.

This is a mechanism result, not evidence of improved performance. In particular,
a forecast overrun does not establish redundant work: the fixtures require
linked traversal, and step-completion timing changes the recorded count. The
artifact gate does not certify a reflection's causal explanation or improve
forecast calibration. Nor does this sample establish that the learned rule
caused the observed transfer regression; it needs its own paired validation.


## Final learning state and validation coverage

Seven reflections were retained, all clean, full-evidence mismatches with confirmed
task goals. Six were processed; one remained held out. The terminal validation
used four new, unprocessed reflections: **two unchanged both-pass pairs and two
inconclusive pairs**, with zero attributed regressions. Four ledger rows and eight
artifact-arm receipts record this stage. Together with the initial gate, the pilot
produced fourteen artifact receipts: twelve passing arms and two inconclusive arms.
Individual arms are not independent efficacy samples.

The inconclusive receipts use the generic refusal/staged-call reason. That guard
also covers unavailable declared checks, whose trace entries are marked denied;
ordinary tool-denial counters alone do not identify the exact refused check.
These cases were not counted as measured-clean or as evidence for retirement.
In particular, paired validation did not establish the cause of revision-6's
observed transfer error. Task-level validation rows intentionally carry no region.

The last consolidation reserved one of four new reflections and processed three.
It produced the same effective rule text; all three candidate probes skipped because
the candidate changed nothing in the recorded situations. The proposal disposition
was `auto_applied_probation`, but finalization preserved the existing rule identity,
scope and previously measured non-probation status while extending its support.
There is **one final active rule, no new second rule, and no retired rules**.
The [final typed rule roster](final-rules.json) is the source for that state;
the proposal label alone is not. No later tasks measured this terminal support update.

The native comparison used eight selection pairs (zero wins, one loss, seven ties)
and four holdout pairs (all ties), and returned **Reject**. That random split is
separate from the explicitly ordered six-task transfer slice. One seed, fixed arm
order, shared server load, and the accuracy ceiling on control transfer limit the
inference. This pilot establishes rule creation, artifact testing and actual later
exposure; it does not establish better planning, better goal interpretation, or
safe generalization to real owner-policy work.

## Evidence and reproduction

[Conditions](conditions.json), [finish audit](finish-audit.json),
[native export](export.json), [native judgement](judge.log),
[sequence summary](summary.json), [artifact scores](artifact-scores.json),
[rule exposure](trace-observations.json), [learning inventory](learning-evidence.json),
[synthetic transcripts](transcripts), [learning records and artifact receipts](learning-evidence),
[stage logs](stage-logs), and [step feedback](step-feedback.json) are archived here.
JSONL records are stored as JSON arrays in the archive; the legacy mined-session
ID ledger is plain text. The original isolated store remains under
`/tmp/mecha-mismatch-bootstrap-20260909/experiments/`.

[Task resources](resources.json) exclude learning-stage calls. Artifact receipts
record their own usage, but neither is a complete end-to-end learning cost ledger.
Cache reuse warnings also make latency comparisons unsuitable for efficacy claims.
The all-target build and warning-free Clippy passed; required-backend workspace
tests passed **2,606**, with zero failures and three intentionally ignored tests.
The strict controlled artifact attribution drill failed as described above; the
separate existing trace retirement drill passed. No learning state was installed
or copied into the operator's home, and no default model or service was changed.

From the repository root, with a fresh isolated bootstrap configuration and empty
learning, session and skill stores:

```bash
MECHA_HOME=/path/to/fresh-bootstrap mecha exp new eval/appraisal-mismatch.toml
MECHA_HOME=/path/to/fresh-bootstrap mecha exp run appraisal-mismatch-qwen36-35b-20260909
MECHA_HOME=/path/to/fresh-bootstrap mecha exp judge appraisal-mismatch-qwen36-35b-20260909
MECHA_HOME=/path/to/fresh-bootstrap mecha exp export appraisal-mismatch-qwen36-35b-20260909 > export.json
python3 scripts/appraisal-traces.py /path/to/fresh-bootstrap/experiments/appraisal-mismatch-qwen36-35b-20260909 trace-observations.json
python3 scripts/appraisal-learning-report.py export.json trace-observations.json
```
