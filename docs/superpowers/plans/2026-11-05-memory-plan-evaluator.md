# Bounded plan evaluation over memory

Approved scope: the user said proceed to the three-step description in chat:
repair goal-context tool use, assess candidate plans against prerequisites/evidence/
concept applicability, and feed structured assessments into automatic revision.
Implement in the existing isolated prototype branch, synthetic-only.

## Design decisions

Reuse `Memory::context` and the resident-only `LocalMemoryModel`. The core owns a
provider-neutral, read-only evaluator and one optional revision round. A thin
`memory evaluate-plans` CLI consumes an explicit JSON file: query, event, goal,
and 1–4 candidates (IDs and bounded step lists). Host CLI supplies synthetic
account/scopes/snapshot. No new tool/provider dependency in the agent loop, no
production integration, actions, approval queue, persistent trusted rules or
nightly process. CLI JSON is an auditable result, not a new database schema.

A quarantined model assesses three fixed dimensions per candidate: prerequisites,
factual support and concept applicability. Each is supported, contradicted or
unknown (concept applicability can be not_applicable only when no concept was
retrieved). Exact quotes address only original episode fields, situation fields,
or explicitly provisional concept fields. Host checks visible citations,
complete/unique candidate/check coverage, limits, and current source scopes and
revisions after each model call. Source quotes prove provenance, not entailment.
Facts/prerequisites cannot be supported solely by a concept. Unknowns require an
explicit information need. Host derives dispositions: contradicted -> revise;
unknown -> needs_evidence; otherwise -> consider. Model ranking must cover every
candidate; only consider candidates can enter the recommendation. This is an
advisory model judgment, never an execution permission or verified plan.

Optional --revise makes one bounded planner call using the assessment and the
same evidence, then reevaluates the proposal. It does not declare the revision
better merely because it was generated. All rounds retain their evidence and
costs. Refuse stale/restricted inputs across inference; do not hold DB transactions
while waiting for a model. No hidden model retry or open-ended reflection loop.

## Tasks and validation

1. GoalMemory: constrain serves to the host-confirmed goal in its schema, explain
   omission/defaulting, and give a host-generated repair hint on mismatch while
   continuing to refuse it. Keep ordinary GoalContext untouched. RED/GREEN tests
   schema and recoverable error, plus existing taint and mismatched-goal checks.
2. Core plan_assessment module: opaque prepared receipt, bounded input, exact
   citations, complete assessments, host-derived dispositions, optional revision
   and reassessment. Tests: customs unknown cannot be recommended; fake quote,
   missing/duplicate check/candidate/ranking rejected; concept-only factual
   support rejected; source mutation/restriction/forget during inference rejected;
   revision receives evaluator output and must be assessed again. No dependencies.
3. CLI evaluate-plans: explicit synthetic input/account/local endpoint/model/holds;
   bounded input read, optional --revise, JSON report with round usage/latency.
   Mock transport integration checks and default-off command behavior retained.
4. Small frozen Spark diagnostic: current resident model, separate binary/home,
   no parallel build/inference or production restart. Cases distinguish missing
   customs prerequisite, an explicitly evidenced feasible alternative, and a
   known contradiction. Record failures without tuning them away. Also exercise
   corrected goal_context schema in a real run.
5. Required fmt/clippy/workspace and standalone tests, one fresh final code review,
   fix important findings RED/GREEN, update existing README/HISTORY/HANDOFF.

## Review focus

No model-selected authority/scope, no trust laundering of concepts or evaluator
text, no implicit approval/action. Complete checklist and ranking validation;
unknown and contradicted inputs cannot become a host recommendation. Exact
quotes vs semantic truth stay distinct. Every consulted source is rechecked after
all inference stages. Input/output budgets and failed/malformed model replies
fail closed. Revision must not drop evidence or skip reevaluation.

## Ledger

- Base: 641e4ca78. Worktree clean. Baseline all-target build and default workspace tests passed.
- Existing chat description is the approved small-feature scope; implementation
  choices above refine it without reopening already answered questions.
- Preflight: GoalMemory schema is the only runtime tool change; evaluator uses
  existing ContextPolicy/ContextPacket and LocalMemoryModel, with no agent-loop
  or source database changes. Fixed assessment dimensions make completeness
  host-checkable; semantic judgments remain model judgments.

- Goal schema/defaulting regression observed RED then GREEN. Guard still refuses mismatched host or caller goal; original GoalContext unchanged.
- Core tests observed missing API RED; concept metadata omission and inconsistent supported-with-questions each reproduced RED then GREEN. Seven core tests cover negative/positive recommendations, revision/reassessment and source changes during inference. Standalone suite 69 unit +2 CLI passed.
- CLI invalid-input regression RED/GREEN; manual loopback integration exercised all 3 stages and rejection/audit of a malformed response. No tools, one message per generation, explicit resident model/seed confirmed in wire requests.
- Strict all-feature clippy passed before the final question-consistency guard; full workspace passed before that guard. Final checks will run again after review.
- Ruling: evaluation is an advisory model judgment, with only citation provenance/checklist coverage mechanically verified. Unknowns cannot be selected, but semantic errors remain possible; no execution or trusted-rule authority is granted.

- Independent final review found no blocking or minor correction requests. Optional future coverage: mutate sources specifically during calls 2/3; current implementation rechecks there and first-call regression passes.
- First Spark run (a55ec6b75): supported route completed all 3 calls and selected Bridge. Concept preparation failed on a source field; known-contradiction assessment emitted field pointers without leading slashes and was rejected. Original artifacts preserved. Runtime fix: enumerate actual visible field pointers in concept/evaluator schemas (RED2/GREEN), without weakening host exact-field checks or leaking gold. Clarified that prerequisites means conditions met/guarded, not merely documented, after a model conflated these. Rerun is disclosed development work, not fresh evidence.
- Reviewer limits retained: no semantic correctness guarantee, production readiness or world-model simulation claim. Parent completes tests/Spark/handoff; nothing delegated away.

- Final checks at 8583fad8f: strict clippy passed; all-feature workspace 5,977 passed/25 ignored; standalone 70 unit+2 CLI. Default workspace 5,892/25 ignored passed before the final feature-only field adjustment. First code review had no blockers/minors; runtime field fix is regression-qualified without a second review loop.
- Spark v2:2/3 cases completed all 3 evaluator calls and chose the expected initial candidate;0 unsafe recommendations among completed cases. Concept case retained a concept then failed closed on a mismatched quotation. No hidden retries. Known contradiction was sometimes labeled unknown; waiting revision introduced an arbitrary24-hour interval. These remain model-quality limits, not waived correctness checks.
- Ruling: preserve and report model-quality failures rather than relax quote checks or tune cases until perfect. Cost: concept-conditioned evaluation is not yet reliable enough for automatic adoption. No production/world-simulator claim; reports remain advisory and point-in-time.

- Goal-context replay complete: Cobalt budget and Vesper current-rule memory-on both correct with complete visible required citations; off arms both wrong. Reused development cases. Spark production fec8d55c7 remains active; separate evaluator 8583fad8f exercised. No action/production restart/model switch/private import.
- All tasks complete within prototype scope. Handoff/usage/architecture/changelog updated against source and preserved reports. No unresolved code-review finding; explicit model-quality limits remain next work.
