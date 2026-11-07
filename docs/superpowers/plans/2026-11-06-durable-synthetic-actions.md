# Durable synthetic actions implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development in separate worktrees. No worker-spawned agents.

**Goal:** Extend the approved durable decision loop through one host-executed synthetic action, an independent observed outcome, deterministic feedback and later recall.
**Architecture:** An explicit v5 store adds one action lifecycle per finalized model decision. A closed core fixture handler supplies actual measurements; the runtime checks the sealed recommendation before execution and stores outcome and feedback separately. CLI remains a thin, explicit synthetic adapter.
**Tech Stack:** Existing Rust, SQLite, serde and prototype CLI; no new dependency or provider.
**Spec:** docs/MECHA-MEMORY-DESIGN.md section 26 and the approved next increment in docs/HANDOFF.md; this plan specifies its bounded implementation.

## Global Constraints

- Default-off memory-prototype; explicit synthetic input; no production or owner-store discovery, migration, services or model switches.
- New explicit v5 creation; v1-v4 behavior remains readable as before, no implicit migration or downgrade.
- At most one action reservation per final model decision. Reserve before effect; an unresolved reservation stays indeterminate and is never replayed.
- Execution requires a current authenticated final recommendation and a separate explicit host fixture binding. Model text cannot choose a tool, path or fixture behavior.
- Original source closure and exact scopes remain bound. Cancellation prevents new work but can retain an actual already-started outcome; forgetting/revocation refuses late content. No transaction spans an effect.
- Content lives in revision-backed records, with complete lifecycle erasure and capture-key suppression. Operational status contains no source text.
- Actual fixture measurements are not model predictions. Feedback uses planning::StepFeedback with goal=None and expected_calls/actual_calls=None; no fabricated task-board goal or production appraisal evidence.
- Parent owns shared docs, integration, Spark/model runs, PR and Site. Workers make no real model calls or deployments.

## Review Focus

- Abstain, withheld, failed, forged or stale decisions cannot execute.
- Concurrent reservations and process loss never duplicate effects.
- Cancellation during work retains only a real outcome; forgetting or restrictions deny late writes and later recall.
- Wrong source expectations remain historical evidence beside a differing actual outcome, not overwritten as truth.
- Unknown/refused outcomes never become successful measurements or a measured policy failure.

### Task 1: Durable action store

**Files:** new mecha-core/src/memory/durable_action.rs and tests; memory/{durable_decision,durable,sql,forget,mod}.rs minimal version/closure changes.
**Interfaces:** Public opaque ActionReceipt::id(), ActionPhase {Ready,Indeterminate,Observed,Finalized,Cancelled,Forgotten}, ActionStatus with model_id, cancelled and spec/observation/feedback RevisionRefs, ActionGuard::check(). Memory::create_durable_action, durable_action_reopen/status/spec/observation/feedback/cancel. Crate-private bind_action(model receipt, spec Value, access), reserve_action, action_guard, settle_action(live permit, host outcome Value, access), finalize_action(feedback Value). Exact signatures follow existing durable_model conventions; send interface details immediately to runtime worker before implementing bodies.
One binding per model decision, full dependency linkage back to model/read siblings. Bind only finalized current model workflow; semantic recommendation validation is runtime-owned. Payload limits 24000 bytes each. Source freshness/auth verified on bind/reserve/guard/settle/finalize; status readable under bound access after revocation. Public arbitrary JSON cannot settle an outcome. Cancelled observed action cannot finalize further feedback. Reuse final content without another execution.
- [x] Add meaningful failing lifecycle tests, including actual observation after cancellation, sibling erasure and no replay.
- [x] Implement explicit v5 schema and operations without changing existing semantics.
- [x] Run focused tests and self-review; commit assigned files and report evidence.

### Task 2: Core runtime and thin CLI

**Files:** new mecha-core/src/memory_action_runtime.rs and tests, lib.rs export; CLI commands/memory.rs.
**Interfaces:** public strict ActionSpec binds a closed fixture version/state, with no executable text; bind(&mut Memory,&ModelReceipt,ActionSpec,&Access), advance/run over ActionReceipt and Access, and status retrieval through store APIs. Use a finite host fixture table for Nori/Arden/Quill, independent from source measurement values. Expose a small host-only boundary usable by process tests; never expose arbitrary model-directed tool execution. Coordinate signatures with store and qualification workers before implementation.
Recompute/verify final decision against retained response and sealed original source using existing decision helper, including request identity; only Recommend can bind. Keep expected_from_evidence optional and distinctly named. Outcome states distinguish observed/refused/unknown; only observed numeric values generate Passed/Failed against original resource/quality constraints, not model self-report. Pure feedback settlement after observed outcome is resumable. Outcome is an Evidence::Observation dependent on action and decision; generic recall should recover it as untrusted source-linked evidence.
CLI explicit init-action and durable-act bind/run/status/cancel commands under --experimental and --synthetic-input. Run uses only closed fixture execution, no endpoint/provider. Reuse final result without reacquisition or model inference. No new production Config fields.
- [x] Write failing runtime and CLI tests for bad decision, actual vs expected values, recovery and no hidden provider.
- [x] Implement runtime/CLI and source-linked recall projection; preserve existing functions.
- [x] Run focused tests and self-review; commit assigned files and report evidence.

### Task 3: Independent qualification

**Files:** new mecha-core/tests/memory_durable_action.rs; optional CLI process tests and prototypes/memory/durable_action_trial.py plus tests.
**Interfaces:** Consume public store/runtime APIs from workers; no product module edits. Coordinate contract directly before writing tests.
- [ ] Implement process loss/concurrency, no-action states, changed/restricted/forgotten sources and cancellation-at-boundary tests; verify actual handler counts.
- [ ] Build a deterministic end-to-end CLI trial using scripted model replies in test setup or an explicitly selected existing frozen model artifact, never invented evidence of fresh model inference. Demonstrate source-expectation mismatch, actual outcome, feedback and later recall; report zero new model calls if none occur.
- [ ] Independently audit dependencies, feedback arithmetic and recall accessibility; report commands and limits.

### Task 4: Integrate, qualify and publish (parent)

- [ ] Review each task for spec/quality, integrate commits, then independent whole-increment review.
- [ ] Run fmt, strict all-feature Clippy, workspace tests and build; isolate any Spark install and verify exact artifact behavior.
- [ ] Update design/architecture/changelog and run repository handoff procedure. Update existing draft PR846, inspect automatic review, keep unmerged.
- [ ] Update living Site with verified implementation, measurements, limitations and next step; verify deployed content.
