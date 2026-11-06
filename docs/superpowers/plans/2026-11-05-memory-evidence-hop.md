# Evidence-informed memory retrieval implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan inline.

**Goal:** Test whether one bounded search hop grounded in initially retrieved episodes improves decisions, with explicit constrained citation selection in every condition.

**Architecture:** Preserve the first comparison and its fixture as a development control. Add a small follow-up module under the existing core evaluation module, reusing capture, access checks, oracle reads, grading and call accounting. The thin CLI adapter passes per-call response schemas to the existing strict local provider. No production store, new dependency, graph schema or runtime cognition change.

**Tech Stack:** Existing Rust/SQLite/serde, QuarantinedPass and llama.cpp JSON-schema responses.

**Spec:** Approved next-step description in chat and `docs/AGENT-MEMORY-SYSTEMS-RESEARCH.md` §7.

## Global constraints

- Same answer prompt/schema and four-record/8,000-byte evidence budget in all conditions.
- Question-only and evidence-informed planners each get one model call and <=2 extra searches; same scope/session/time/snapshot filters. Call/search/output caps match; input token use is measured, not assumed equal.
- Both retrieval conditions perform the same initial recall. Only the evidence-informed planner receives those results; no answer choices, expected IDs or gold outcomes enter either planner.
- Shared final retrieval gives anchors priority over context neighbors, with rank fusion within each group. This policy applies equally to both conditions; old comparison stays reproducible.
- Each answer schema enumerates all and only visible citation IDs and public choice keys. Host grading still checks duplicate/unsupported/missing citations, choices and abstention; no repairs or silent normalization.
- Default-off synthetic prototype. Development uses the existing16 cases. Freeze12 newly authored cases before model runs; use separate fresh databases and report split/fixture hash/protocol.
- Strict resident-model-only local transport, no tools, no config discovery, no retries. Preserve source taint, guards and budgets.
- First verify development/citation behavior, then run the held-out split once with seed7 on Spark; preserve negative results. This is a controlled synthetic diagnostic, not a leaderboard claim.

## Review focus

1. Schema generation must not reveal gold or encode a privileged answer, including empty-evidence cases.
2. Initial planner evidence and follow-up hits remain authorized under the same filters, and both evidence arrays obey the byte cap.
3. Explicit ID lists are enforced on the wire and in host grading; malformed/extra/duplicate IDs cannot silently pass.
4. Retrieval must keep query anchors ahead of incidental context without giving one condition different ranking/budget rules.
5. Fixtures must have feasible correct choices, sufficient oracle premises, genuinely absent answers, paired contexts, and no reuse of development session IDs.

### Task1: Core protocol and deterministic tests

Files: `mecha-core/src/memory/model_followup.rs`, `model_followup_tests.rs`, `fixtures/model_followup.json`; small shared changes in `model_evaluation.rs`.

Interfaces: `followup::run(path,model,Split,complete,progress)`, callback consumes `ModelRequest {prompt,response_schema}`; rows retain initial evidence, final evidence, queries, call schemas, scores and costs. `Split::{Development,Holdout}` selects embedded fixtures only.

- [x] Write failing tests for evidence visibility, exact schema enums, empty evidence, extra IDs, query caps, anchor selection, shared filters, matched call counts and fixture integrity.
- [x] Implement the core protocol and run standalone tests RED→GREEN; keep legacy tests passing.

### Task2: CLI, review and measurement

Files: existing memory CLI/test files; prototype README and existing architecture/research/history/handoff docs.

- [x] Add `evaluate-model --follow-evidence [--development]`; default follow-up split is holdout. Preserve original command behavior without the flag. Add wire-contract assertions for schemas and call counts.
- [x] Run targeted tests, required workspace verification and fresh code review. Correct important findings with regression tests before inference.
- [x] Commit and transfer to isolated Spark checkout; use the update skill. Build before inference and verify the installed binary's new flags.
- [x] Run development and then frozen holdout with the resident model. Retain reports/source hashes, summarize decision correctness, strict grounding, evidence coverage, abstention, token use and latency. Do not tune on held-out results.
- [x] Apply handoff skill, record remaining gaps, commit docs and leave branch/worktree intact and unmerged.


Completion: source `27e15d8a2` installed in an isolated Spark path; development
and frozen holdout completed with no ID/parse/transport violations. Research §8
records unchanged accuracy (8/12 in each search arm), strict grounding (3/12 vs
5/12), oracle (12/12 correct,10/12 grounded), costs and limitations. Fresh review
had no Critical/Important issues; dedicated duplicate-valid-ID test deferred as
Minor, production-scale and generic Usage missingness remain outside this increment.
Branch/worktree preserved unmerged; no production or private-data migration.
