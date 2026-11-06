# Automatic memory name bindings

> **For agentic workers:** use superpowers:executing-plans inline; one fresh final reviewer.

**Goal:** Test automatic name selection and the causal retrieval effect of enforcing the same selected names.
**Architecture:** New synthetic core comparison nested under model_followup, reusing capture, search, fusion, answer schemas, grading and local transport. One schema-constrained planner per case produces <=2 {query,mention,source_id} records. Same plan goes to ordinary and required-mention retrieval; third arm supplies oracle evidence. Planner calls recorded once, answer calls per arm. Add --auto-bindings (exclusive of --follow-evidence) and allow --development with either protocol.
**Tech stack:** existing Rust/SQLite/serde; no dependencies or migrations.
**Spec:** user's approved next-step description, research §9.

## Constraints / review focus

- Name/source both null or both present. Bound name <=256 bytes/32 tokens, literal phrase in original text field of the cited initial record AND in query. Unicode61 same as recall_mention. Reject invalid plans for both search arms; no quiet broad fallback. Oracle can still run.
- All initial IDs allowed in planner schema; no gold, choices, target names, oracle records or expected scores enter planner input. Membership establishes provenance only, not usefulness.
- Shared initial recall and plan; same <=3 searches, two anchors/search, four final records/8000 bytes; only enforcement differs. Counterbalance answer-arm order. Cost totals count shared planner once, not three times.
- Preserve all scope/account/session/time/snapshot/taint gates. Legacy experiments unchanged.
- Report planner error, grounded binding count and authored target-name recall separately from evidence coverage, choice correctness, exact support/format compliance, tokens/latency. Exact support scoring remains frozen; no silent repairs.
- Development uses prior12 cases. Freeze14 fresh cases (8 changed-goal,2 corrections,2 temporal,2 absent) before any inference. Include multiword, diacritic and stem-collision names. Independent review checks all gold feasibility premises.
- Full tests and isolated Spark build before development then seed7 holdout; resident local model, no tools/retries/model switch. Preserve negative results. No production/private-data/nightly migration.

## Task 1: Core and fixture
- [x] Baseline build/workspace tests. RED tests for hallucinated/wrong-source names, null inconsistency, empty/oversize/manyqueries, Unicode matching, shared planner call/count/schema/filter budget, plan-failure isolation and no gold leakage.
- [x] Implement binding_evaluation.rs + tests and fresh model_bindings.json. Extract shared phrase validation from recall with regression coverage. Add target names solely for posthoc diagnostic scoring. Standalone GREEN.

## Task 2: CLI and qualification
- [x] RED real-binary --auto-bindings test; implement thin adapter routing with schema transport shared. Verify legacy paths and CLI flag combinations.
- [x] Required fmt/clippy/workspace tests plus fresh final code/fixture review. Fix Important findings with regression tests.
- [x] Commit source, transfer/build/install isolated Spark prototype using update skill; run development then frozen fresh cases. Retain exact source/fixture/binary hashes, reports and costs. Never tune holdout after viewing results.
- [x] Update existing research/architecture/README/changelog and scoped handoff/history; commit docs; leave branch/worktree unmerged.

Completed source83753f9ea, installed only in Spark memory-bindings-20261105. Fresh14cases: unbound9/14 correct, bound13/14, oracle14/14; complete support7/12→11/12. One invalid plan per split, preserved with no fallback. Full results/costs/limits in research§10.5950 workspace passed/25 ignored,50 standalone,10 CLI, strict lint/fmt. Review had no blockers; nonnull-routing regression and explicit countfield deferred Minor. Branch remains unmerged.
