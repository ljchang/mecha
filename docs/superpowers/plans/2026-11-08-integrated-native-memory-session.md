# Integrated Native Memory Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Connect an explicit native test session to persisted recall, provisional concept learning and evidence-preserving revision.

**Architecture:** Extend the existing situated task executor and introduce explicitly guarded concept admission. Reuse SQLite, LocalAuthority, GoalMemory, Agent::run_in and the current concept/rumination format. The CLI remains a thin adapter.

**Tech Stack:** Rust, SQLite/rusqlite, native JSONL sessions, existing local provider and router holds.

**Spec:** docs/MECHA-MEMORY-DESIGN.md §31.16 and the user's approved integrated prototype milestone.

## Global Constraints

- Explicit synthetic Test sessions only; no production activation, owner-graph merger or nightly migration.
- Actual action output is independently checked and never overwritten by an expected fixture value.
- Reuse the existing executor, path jail, taint and source authority; no new instruction authority.
- Missing authority, stale evidence, source withdrawal and ambiguous provenance fail closed.
- Preserve complete dependencies and scope restrictions, including uncited input and metadata used to identify episodes.
- Legacy concept APIs remain closed to externally bound sources.
- No automatic replay of started execution; model failures remain recorded failures.
- No personal source material in tracked files. Strict Spark privacy scan before commits/pushes.
- Separate worker worktrees; parent owns integration and single-writer documentation. PR remains draft, no merge.

## Review Focus

1. Multiple native passages or prefixes must not masquerade as independent experiences.
2. Revoked transitive or uncited evidence must invalidate generation and revision, including no-write outcomes.
3. A successful tool call must not stand in for a correct observed artifact.
4. Restart or cancellation must not repeat a started action or silently omit its partial record.
5. Private needs and authority files must stay outside the model workspace and memory payload.

### Task 1: Explicit guarded concept admission

**Files:** memory/concept.rs, memory/concept_rumination.rs, memory/host_capture.rs, focused native concept tests.

**Interfaces:** Add Memory::prepare_native_concept / retain_native_concept with the same parameters and return types as their existing counterparts. Add prepare_native_concept_rumination / finish_native_concept_rumination with their existing counterpart signatures. Native references name canonical receipt originals. Keep request mode private and reject crossed finish APIs.

- [ ] Write and observe failing tests for independent native sessions, same-session refusal, projection refusal and legacy refusal.
- [ ] Resolve canonical original pointer/text/kind/session through the active host receipt; retain projection metadata dependencies and full source closure.
- [ ] Implement explicit guarded preparation/retention and all three rumination outcomes; reject interpreted ancestors initially.
- [ ] Test withdrawal, forgetting, transitive scope mutation, freshness, missing guard and source changes across inference; preserve opposition and concept-only restrictions.
- [ ] Run focused tests, report patch and independent task review; commit only after strict privacy scan.

### Task 2: Reusable native session runner and thin CLI

**Files:** memory_situated_workflow.rs, mecha-cli/src/commands/memory.rs, focused session tests.

**Interfaces:** Host-owned NativeSessionRequest and provider-neutral run_native_session over explicit database, authority, access and new task root. Preserve run_planning_task as an unchanged legacy fixture adapter. Expose one explicit native-session CLI command using local-only provider/holds and a new v8 store initialization path if needed.

- [ ] Write and observe failing actual-artifact/reopen tests before generalizing execute.
- [ ] Bound prompt, initial file set and relative paths; use real FsRead/FsWrite, frozen criterion, GoalMemory and capture_with_evidence.
- [ ] Persist started state before provider discovery and refuse existing task root; preserve partial session, cancellation and independent outcome.
- [ ] Test actual bytes/recorded invocation, jail escape, authority isolation, current goal binding, needs non-disclosure, restart and duplicate execution refusal.
- [ ] Run focused tests and independent task review, preserving old situated tests.

### Task 3: Integrated acceptance and qualification

**Files:** one focused integration test/example and prototype usage documentation; parent maintained design/HANDOFF/HISTORY/CHANGELOG and private Site.

**Interfaces:** Tasks 1 and 2 public APIs; no new executor or store.

- [ ] Run two genuinely separate native supporting sessions, close/reopen and verify relevant recall-informed action and observed artifact.
- [ ] Learn a provisional concept from canonical originals; execute an in-scope counterexample, revise once and reopen to verify opposition and complete source closure.
- [ ] Verify changed-goal interpretation separately from contradiction, and withdrawal/forgetting after reuse.
- [ ] Run required build, strict Clippy, default/memory-enabled tests and appropriate actual-binary checks on isolated Spark.
- [ ] Independently review the combined change and address substantive findings.
- [ ] Run handoff, strict Spark privacy scans; update draft PR and private Site with verified outcomes and limits, then close exact-head CI/review loop.

Real-model qualification, if run, uses the resident owner-selected model, existing holds/no-autoload transport, fixed bounded calls and durable first-attempt reports. It must not convert authored acceptance into a claim of model reliability.
