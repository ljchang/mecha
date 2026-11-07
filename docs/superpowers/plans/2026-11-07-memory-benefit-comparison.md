# Memory benefit comparison implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. The user approved execution and parallel review; no further design approval is pending.

**Goal:** Measure 48 new first responses under the existing v4 outcome-recall protocol, separating evidence-relative correctness from benefit of a recalled actual.

**Architecture:** A separately versioned Python runner reuses the existing CLI and durable synthetic fixture lifecycle. No Rust behavior, storage schema, production configuration, owner data or earlier experiment is changed. Fixtures and all requests are frozen before real inference; authored HTTP responses qualify the whole schedule first.

**Tech stack:** Python standard library, existing experimental Mecha CLI, isolated Spark SQLite fixtures and resident local model.

**Spec:** `results/memory-provenance-20261107/next-evaluation-proposal.md` (operator-local), with exact cases below retained here for portability.

## Global constraints

- 48 first responses: six cases × seeds 71/83 × source_only/source_and_outcome × fast/thinking2048.
- Core protocol outcome-reappraisal-v4; format feasibility-provenance; max output 8192 and reason 4000 characters. No model load, retries, repair or replay of uncertain dispatches.
- Source tuples below are (minutes, energy, quality), ordered Nori, Arden, Quill. Actuals come only through durable-act baseline, never inserted directly.

| Case | Source tuples | Objective | Budget | Quality | Actual method | Source optimum | Outcome-informed optimum |
| --- | --- | --- | --- | --- | --- | --- | --- |
| boundaries | (4,2,8); (8,7,9); (6,5,7) | minutes | 3 | 9 | Arden | abstain | Arden |
| energy-correction | (4,2,8); (8,3,9); (6,5,7) | minutes | 6 | 7 | Nori | Nori | Quill |
| quality-alternative | (4,9,8); (8,3,9); (6,5,10) | minutes | 6 | 9 | Quill | Quill | Arden |
| emptied | (4,9,8); (8,3,8); (6,5,10) | minutes | 5 | 9 | Quill | Quill | abstain |
| unchanged | (4,2,8); (8,7,9); (6,5,7) | minutes | 6 | 7 | Arden | Nori | Nori |
| energy-tie | (4,4,8); (8,4,9); (6,5,8) | energy | 5 | 8 | Arden | Nori or Arden | Arden |

- Simulator tuples: Nori (4,9,8), Arden (8,3,9), Quill (6,5,6). Shared outcome-informed reference replaces ONLY the retained method in source values; it is not full simulator truth. Hidden simulator optimum is secondary.
- Keep raw replies, first-response errors, checks, citations, prose, withholding, tokens and latency separate. Never use host acceptance alone as success.
- Counterbalance treatment order by case/seed and alternate evidence order. Freeze schedule, source revisions, snapshots, request and schema order, binary/helper/scorer hashes and resident identity/settings before calls.
- Parent owns real inference, PR, Site, HANDOFF/HISTORY; implementer owns runner and tests in an isolated worktree. No production deployment or merge.

## Review focus

- Tie-aware energy objective must not inherit the old minutes-only oracle.
- Missing effective citations must fail; superseded expectation omission must pass; no gold leaks into requests.
- Source-only abstention can be correct with available evidence yet lose under outcome-informed reference.
- Partial/failed/uncertain calls cannot be retried or silently excluded from denominators.
- Drift in fixture source, stored records, executable, helper or resident settings must prevent unqualified execution.

### Task 1: Versioned runner and qualification

**Files:** Create `prototypes/memory/memory_benefit_trial.py` and `prototypes/memory/test_memory_benefit_trial.py`; reuse existing outcome/constraint/durable helpers without altering their frozen behavior.

**Interfaces:** `frozen_cases()`, `planned_pairs()`, `expected(case, arm, simulator=False)`, `score_row(...)`, `create_fixtures(binary, output)`, `run_trial(...)`, `qualify_scripted(...)`; CLI patterned after constraint_abstention_trial.py. New protocol memory-benefit-pilot-v1.

- [ ] Write failing tests for all six exact oracles, both tie options, 24 unique paired slots/48 responses, balanced orders, effective citation policy, malformed/false checks and incomplete report accounting.
- [ ] Implement fresh fixture lineages with authored prerequisite decisions that select each designated method under a valid temporary contract. Reuse lineage where identical source/actual permits. Assert actuals match baseline handler.
- [ ] Implement separate evidence-relative, shared outcome-informed and hidden-simulator choice metrics; paired improvement/regression/unchanged counts; abstention/check/citation/host/cost metrics.
- [ ] Freeze exact preparations before calls; verify full scripted HTTP request schedule and schema order, no retries or output overwrite, post-audit unchanged fixture/model state.
- [ ] Run unit tests and authored full-schedule qualification against installed isolated Spark binary. Preserve evidence; do not invoke real model.
- [ ] Independent spec/code review, resolve material findings, integrate commit.

### Task 2: Frozen real comparison and reporting

**Files:** Operator-local `results/memory-benefit-20261107/`; existing `prototypes/memory/README.md`, `docs/MECHA-MEMORY-DESIGN.md`, `docs/HANDOFF.md`, `docs/HISTORY.md`; existing private Site RESEARCH.md.

**Interfaces:** Task 1 frozen manifest and per-row reports feed aggregate and independent explanation audit.

- [ ] Freeze explanation rubric before real inference: correct effective evidence, inclusive inequalities, objective/tie handling, no invented numbers, expectation/actual distinction. Reasoning prose is separately graded and untrusted.
- [ ] Verify baseline and authored qualification, artifact hashes and resident model; execute all 48 first responses once, retaining errors and uncertain slots without replay.
- [ ] Independently audit frozen artifacts and raw answers; calculate paired benefit, reasoning correctness, host acceptance, citations, explanations and cost. Report six authored cases/two seeds as a small diagnostic, not general efficacy or v4 superiority.
- [ ] Run required checks and review, update PR #846 and existing owner-private Site with verified results and remaining limitations; retain raw results on Spark and locally.
