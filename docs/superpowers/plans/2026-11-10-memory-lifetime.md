# Ordinary-session chronological learning evaluation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Connect ordinary-session memory to bounded native concept learning and compare three persistent-memory treatments on a repeatable chronological cohort.

**Architecture:** Core owns guarded source selection and native concept preparation/retention. A thin host CLI invokes the existing quarantined local model transport. An external Python adapter runs ordinary sessions chronologically and scores artifacts independently, preserving failures and costs.

**Tech Stack:** Rust, existing SQLCipher memory prototype and local provider, Python standard library, JSON fixtures and scorecards.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.21–31.23.

## Global Constraints

- Keep `memory-prototype` default-off and all existing protocols unchanged.
- Three initially empty arms: off; episodic recall with frozen concept learning; episodic recall with adaptive concept learning.
- Core owns authority and learning; Python owns benchmark orchestration and independent scoring.
- One explicit learning operation permits at most one local model request, with no retry, model switch, tool access or automatic nightly scheduling.
- Recovery-only handles cannot learn. Independent observation sessions must not call recall.
- Future episodes and private answers never enter current model inputs. Missing/failed slots remain in the denominator.
- No owner graph, production memory, production deployment or nightly migration.

## Review Focus

- Source withdrawal during unsuccessful inference must still be observed (Task 1).
- Recovery handles and foreign/derived originals must not acquire learning authority (Task 1).
- Interrupted slots cannot silently replay actions or spend another model call (Task 2).
- Failed learning must not disappear from costs or change the planned decision denominator (Task 2).
- Current workspace content and recall delivery must not expose future observations or private gold (Task 2).

### Task 1: Guarded ordinary-session learning operations

**Files:** `mecha-core/src/memory_session.rs`, new `mecha-core/src/memory_session/learning.rs` and focused tests; `mecha-cli/src/commands/memory_session.rs`, `mecha-cli/src/commands/memory.rs`, `mecha-cli/src/main.rs`.

**Interfaces:** Add bounded host source descriptors selected by retained run ID and original index. Expose explicit induction and rumination through SessionMemory, retaining opaque native preparation receipts across the single model callback. The CLI accepts an owner-selected learning JSON input, explicit workspace, endpoint, model, holds directory and seed; it emits a versioned JSON report including generation input, raw reply, usage, status and retained revision or rumination outcome. Keep all parsing and authority validation in core. Existing init/retry/forget commands remain compatible. Pin the exact JSON contract in tests and report it to Task 2 before that task starts.

- [x] Write failing tests for independent ordinary captures → induction → reopen → exact independent opposition → revision; recovery/foreign inputs denied; source withdrawal on success and failure denied.
- [x] Run focused tests on Spark and preserve the initial failing result.
- [x] Implement guarded source resolution and one-call learning using native APIs, without exposing raw Authority or bypassing current-source checks.
- [x] Add thin asynchronous CLI dispatch using the shared loopback-only, no-autoload model transport. Preserve failed replies and explicit unknown usage.
- [x] Run focused Rust and CLI compatibility tests, then obtain task review before Task 2.

### Task 2: Frozen chronology, ordinary CLI adapter and independent scorecard

**Files:** new `prototypes/memory/ordinary_lifetime.py`, `prototypes/memory/test_ordinary_lifetime.py`, versioned public/gold fixtures under `prototypes/memory/fixtures/`; a small recording transport helper if required for no-autoload ordinary provider calls.

**Interfaces:** Runner `freeze`, `run` and `score` commands consume an explicit binary, output directory, loopback endpoint/model and seeds. Freeze records all arm/task/learning slots before execution. Run invokes ordinary `mecha run` and Task 1 host learning commands. Score consumes frozen host gold plus actual artifact/output/usage records; it never supplies gold to run. Outputs `manifest.json`, append-only slot events, per-slot artifacts, and `scorecard.json` with protocol and fixture/scorer identities.

- [x] Write failing tests for arm isolation, chronology-only task materialization, fixed planned slots, unknown usage, absent/malformed artifacts, interrupted-slot refusal and paired phase scoring.
- [x] Run Python tests and preserve RED evidence.
- [x] Implement a bounded cohort with two independent observations, two transfer decisions, one independent counterexample and two later decisions. Use the same current task contract in all arms, with only declared memory/learning treatment changes. Limit adaptive learning to one induction and one revision; record abstention unchanged distinctly.
- [x] Run authored subprocess/transport tests proving actual CLI argument and output contracts and leakage boundaries. No model-generated success label is a grade.
- [x] Obtain task review; freeze and execute the three-arm development comparison on the resident Spark model. Preserve all attempts, failures, missing usage and source/packet evidence. Report results without treating repeated seeds as independent families.

### Task 3: Qualification, publication and handoff

**Files:** existing memory design, ARCHITECTURE, HANDOFF, HISTORY, CHANGELOG and prototype user documentation; existing Sites project sources.

- [x] Run required workspace build/tests, fmt, strict all-feature Clippy and relevant prototype suites with one Cargo writer per machine.
- [x] Review the complete change; resolve verified load-bearing findings with scoped regression tests.
- [ ] Update compact Site capability/status and detailed research archive with actual results and remaining gaps; do not expand the homepage into a work log.
- [ ] Run strict source and exact-message privacy scans on Spark before normal commit/push to draft PR #846; check exact-head CI and actual automated review.
- [ ] Run scoped handoff verification against source. Leave production deployment, owner memory, broader activation and merge pending.
