# Comparison Provenance Implementation Plan

**Goal:** Separate host comparison provenance from model citation claims without repairing model replies.
**Architecture:** Opt-in outcome-reappraisal-v4 over the existing guarded, read-only synthetic pair; existing v1/v3 behavior stays unchanged.
**Tech stack:** Rust core/CLI, SQLite, authored HTTP qualification in Python.
**Spec:** MECHA-MEMORY-DESIGN.md section29 and the user's continuing authorization.

## Global constraints

Preserve v1/v3 request bytes, report fields, defaults and historical scores. Raw model citations and prose remain unchanged. Require all effective measurement citations, exact candidate checks, existing source/access guards and local-only transport. Host provenance never becomes model evidence or action permission. No new persistence schema, dependency, executor, production service change or owner-data migration. Keep PR846 draft and the Site audience unchanged.

## Task 1: core and CLI

Single code implementer owns memory_outcome_reappraisal.rs, its tests, memory/experience_checks.rs and mecha-cli/src/commands/memory.rs. Add ResponseFormat::FeasibilityProvenance (CLI feasibility-provenance), protocol v4. Reuse feasibility-first schema/order/parser and reason bounds. Its prompt requires citations for each effective measurement including rejected alternatives, while additional citations are allowed and original expectations remain supplied. Explicitly distinguish host packet provenance from model claims.

Factor a narrowly named effective-measurement citation assessment entry point; keep assess_measurements as the unchanged all-passages wrapper. Do not forge a Choice or filtered Request to bypass a check. Keep existing arithmetic/advisory suboptimal-choice and unnecessary-abstention semantics; do not introduce another policy change. V4 abstention must cite all effective sources too. Candidate source/value/feasibility claims must still match independently derived values exactly.

Only v4 reports add citation_policy=effective-measurements-v1 and a comparison_provenance audit: all supplied source IDs, revision references, original/projection hashes, host roles (effective expectation, superseded expectation, effective actual, non-numeric actual); method-to-effective-source mapping; model-omitted packet IDs; missing effective citation IDs. The audit must derive from frozen passages and host arithmetic, never model checks. Mark reason_status=unverified_model_prose. No original/projection text duplication is needed. Keep raw citations as submitted and all original call replies.

Write failing tests before implementation: omitted superseded expectation v3 withheld/v4 eligible with raw citations intact; missing effective alternative/actual withheld including abstention; expectation cannot substitute for actual; false checks with all citations withheld; refused/unknown actual does not override; true abstention and unnecessary-abstention finding preserved; exact legacy serialization; v4 source invalidation fails closed. Run focused tests and fmt. Commit only owned files; report red/green commands/results and concerns. No subagents or remote inference.

## Task 2: actual CLI qualification

After Task1 review, a fresh worker adds a bounded authored-HTTP qualification test for v4 using existing fixture/server utilities, with zero real inference. Verify CLI option/protocol, real transmitted schema order, exact submitted citations, report source revisions/hashes and effective roles, superseded omission acceptance versus missing-effective/false-check withholding. Preserve existing runner and old rubrics. Parent runs the test against an isolated Spark binary; missing fixtures must be explicit skips locally and zero skips in Spark qualification.

## Task 3: integration and delivery

Parent runs required fmt, strict Clippy, workspace default/all-feature tests, standalone and Python checks on Spark, installs/verifies an isolated prototype artifact, and runs authored HTTP qualification. Independent final review covers this whole increment and its interfaces. This increment establishes provenance bookkeeping, not a model-reasoning improvement: broader multi-seed real-model evaluation remains next, with new protocols/schedules frozen before calls and old results unchanged. Update existing docs, handoff/history, changelog, prototype guide and living Site. Push PR846 coherently and inspect CI/automated review. Do not merge.
