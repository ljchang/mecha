# Durable model decisions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Execute the approved work in isolated worktrees; no worker-spawned agents.

**Goal:** Make one bounded model decision and optional revision durable after a real guarded synthetic read.
**Architecture:** A v4 attempt ledger owns allowances and linked content. Pure decision helpers reuse existing arithmetic; a core runtime drives one initial and optional revision through an explicit callback. The thin CLI uses the existing local quarantined provider.
**Tech Stack:** Rust, rusqlite/SQLite, existing Tokio/provider interfaces; Python pilot orchestration.
**Spec:** docs/MECHA-MEMORY-DESIGN.md §25 (approved conversation scope; implementation choices recorded below).

## Global Constraints

- Default-off memory-prototype; explicit synthetic input; no production/owner-store discovery or service changes.
- Initial and retry each at most once; no replay of unresolved calls; no new transport retries/tools/provider switches.
- New explicit v4 creation, no implicit migrations. All original sources and exact scopes remain bound.
- Content in linked revisions, complete erasure/suppression; no trusted outcome from model JSON.
- Parent owns CLI/runtime integration, CI coverage fix, shared docs, Spark/model runs, PR and Site.
- Parallel workers use their own managed worktrees; coordinate shared Cargo target use. No worker model calls.

## Review Focus

- Cancel/forget/source restriction during provider preflight and calls must not grant a later decision.
- Crash after raw response settlement must permit pure reassessment without another inference.
- Failed/unknown/refused/duplicate JSON cannot become an accepted recommendation or retry loop.
- A valid bounded read prefix is not evidence of whole-file completeness.
- Old v3 finish cannot write a v4 decision, and generic captures cannot resurrect erased records.

### Task 1: Durable model attempt store

**Files:** new memory/durable_decision.rs and durable_decision_tests.rs; modify memory/durable.rs, sql.rs, forget.rs, mod.rs. Paths under mecha-core/src/.
**Interfaces:** public opaque ModelReceipt with id(); AttemptKind {Initial,Retry}; ModelPhase {Active,Finalized,Cancelled,Forgotten}; AttemptPhase {Unused,Indeterminate,Settled,Assessed}; ModelStatus with id, phase, read_id, contract/decision refs, initial/retry AttemptStatus (phase, request/result/assessment refs), cancelled. Public ModelGuard with private construction and check()->Result<()>.
Public Memory operations create_durable_model(path,domain), durable_model_reopen(id,access), durable_model_status(receipt,access), durable_model_spec(receipt,access)->Value, durable_model_attempt(receipt,kind,access)->Option<AttemptData>, durable_model_decision(receipt,access)->Option<Value>, durable_model_cancel(receipt,access). AttemptData contains request Value, optional result Value and optional assessment Value; metadata refs available through status.
Crate-private operations durable_bind_model(read_receipt,spec:Value,access)->ModelReceipt; durable_model_read(receipt,access)->ModelRead {task:RevisionRef,observation:RevisionRef,content:String,sha256:String}; reserve_model_attempt(receipt,kind,request:Value,access)->ModelPermit; model_attempt_guard(permit,access)->ModelGuard; settle_model_attempt(permit,HostModelResult,access)->ModelStatus; assess_model_attempt(receipt,kind,assessment:Value,retry_requested:bool,access)->ModelStatus; finalize_model(receipt,decision:Value,access)->ModelStatus. HostModelResult::new(Value) is crate-only, no Deserialize. Payload records reply/error/timing from host, never model-authored assessment. All receipt/permit arguments borrowed except consumed permit at settlement. Normal parameter convention &Access, &ModelReceipt.
One binding per underlying read. Bind only successful current observation, before any legacy proposal. Seal arbitrary spec bytes (semantic validation parent-owned); ModelRead checks successful provenance and returns exact stored content. Request <=48000 serialized bytes, result <=64000, assessment/final/spec <=24000 each. Errors oversized are bounded at runtime before settlement with explicit failure metadata, never truncated into valid JSON. No transaction over I/O. Cancelled calls may settle raw result but not assess; stale/forgotten sources refuse settlement. Retry reservation requires initial assessed retry_requested=true; retry assessment cannot request third call. Finalization requires last appropriate assessment, no unresolved slot. Forgiving status reads remain content-free under bound access, even forgotten.
- [x] Write failing store tests: v3 compatibility/v4 refusal by old finish, unique binding, two slots, reserve-before-effect, settlement/reassessment recovery, concurrent reservation, cancellation after reserve, source restriction/change/forget, sibling/raw erasure and suppression.
- [x] Implement atomic operations and short read guards; extend version/forget gates without downgrade.
- [x] Run standalone memory tests and focused checks; inspect diff; commit only assigned files. Report commands, counts and limitations.

### Task 2: Pure decision contract and shared evaluator

**Files:** new memory/decision.rs and decision_tests.rs; modify experience.rs, experience_checks.rs and mod.rs minimally.
**Interfaces:** Objective {MinimizeEnergy,MinimizeMinutes}; Contract {workload:String,objective:Objective,energy_budget:u32,minimum_quality:u32}; EvidenceView {None,Observed}; serialize/deserialize ordinary contract data with deny_unknown_fields and bounded workload (1..128 bytes). ReadBinding crate-only {task,observation:RevisionRef,read_sha256:String}. prepare(&Contract,ReadBinding,&str,EvidenceView)->Result<Prepared> crate-only. Prepared no public ctor/Deserialize: request()->&experience::Request, request_sha256()->&str, assess(&str)->Result<Assessment>, reconsideration(&str)->Result<Option<Request>>, finish(&str)->Result<Decision>. Decision Serialize exposes proposed_choice, citations, reason, assessment, disposition {Recommend,Abstain,Withheld}, original binding/request hash. Assessment and ordinary response data may gain required data-only derives.
Envelope exactly {kind:"host_authored_synthetic_measurements_v1", workload, observations:[{choice,observed_minutes,energy,quality}], complete:true}. Entire received JSON through recursive UniqueValue then deny_unknown_fields; <=3 unique closed methods Nori/Arden/Quill; u32 values. Parse full source even None view; no-memory supplies zero projected passages. Existing checker/prompt/schema behavior remains byte-compatible for old trials. Extract pure review preparation and assessment helpers; async reconsider wraps them with same bounded call behavior. No hidden EFFECTS, World::best, inference, persistence, or whole-file completeness claim.
- [x] Add failing parser/view/projection/binding/choice/retry tests; include duplicate nested members, malformed/truncated bytes, wrong workload, missing alternatives, tie, unknown fields, refusal handled as outer runtime error.
- [x] Factor shared evaluator and implement immutable Prepared/Decision.
- [x] Run new tests and existing experience tests unchanged; commit assigned files and report evidence.

### Task 3: Runtime, CLI and CI (parent)

**Files:** new mecha-core/src/memory_decision_runtime.rs + tests, crate-root lib.rs; CLI commands/memory.rs; .github/workflows/ci.yml.
Public core ModelSpec {contract:decision::Contract,model:String,seed:u64,view:EvidenceView}; the internal sealed wrapper fixes checker_version; bind(&mut Memory,&durable::Receipt,ModelSpec,&Access)->Result<ModelReceipt>; async decide(&mut Memory,&ModelReceipt,&Access,infer)->Result<ModelStatus>, infer callback FnMut(String,Option<Value>,ModelGuard)->Future<Result<ModelReply>>. Public advance() has the same signature and performs exactly one durable step (inference+raw settlement, pure assessment, or finalization); decide() loops the same step until terminal or indeterminate. This supports cooperative scheduling and real process-boundary qualification without test-only hooks. Core binds/validates source, reserves exact request+schema+model, then gives guard to the explicit caller. CLI checks guard after awaited local-model preflight immediately before provider.complete; no new model/provider discovery. Store raw response/error/millis independently; deterministic assessment recomputed from sealed source and checked stored request on resume. Reuse finalized results; cancel/forgotten never invoke model. Serialize failures as disposition failed, no new model call for malformed responses.
CLI explicit init-model, durable-decide bind (read ID, contract JSON, model/seed/view), run (task ID, endpoint/holds directory), status/cancel. Name can follow existing command conventions; no silent acquisition/reread. Fixed local model settings retained. Fresh result remains a recommendation, no selected-method execution.
- [x] Add callback-count/restart/failure tests before runtime, parser tests before CLI.
- [x] Add pinned Ubuntu CI step cargo test --workspace --locked --features memory-prototype per PR review finding.
- [x] Integrate worker commits, run required checks, isolated Spark build/install and pilot.

### Task 4: Independent qualification and frozen pilot

**Files:** new mecha-core/tests/memory_durable_model.rs; optional mecha-cli/tests/memory_durable_model_cli.rs; prototypes/memory/durable_model_trial.py and focused runner tests/fixtures.
Consumes public parent runtime and Memory APIs above; no product module edits. Callback can check ModelGuard before returning scripted ModelReply; subprocess tests kill actual worker around named barriers without adding production fault-injection flags. Pilot uses explicit CLI commands and isolated new DBs/files, frozen source fixtures, no private input. Eight valid task pairs: reversed objective (2), energy/quality constraint (2), missing alternative (1), none feasible (1), tied optimum (1), fresh value combination (1). Two views per task, max16 initial +16 optional revision calls. No-memory often should abstain. Report initial supported-optimal choices, appropriate abstention, host withholding, failures, citation coverage, optional revision gains/costs separately; max-call and source-manifest audit. Malformed/adversarial source tests are deterministic rather than mislabeled fixed-call pilot rows. Parent alone executes real model calls.
- [x] Test reservation/process loss, settled-response recovery, zero replay, cancellation/forget/source changes during both slots, malformed/refused/wrong-model calls, raw erasure and distinctions among final states.
- [x] Implement frozen runner and tests; verify no-source and observed arms use same goal/schema/model/settings and independent stores.
- [x] Run focused independent checks after integration, report limitations, commit assigned files.


Implementation status (2026-11-06): tasks integrated at `3ed543a17`; independent
review found the status-after-source-revision and invalid-pilot-reply cases, both
fixed with regression coverage. Full Spark qualification/publication evidence is
recorded in HISTORY and HANDOFF when complete. Tests preceded the store/decision
implementations; parent runtime tests were written first but could only compile
once those interfaces arrived. The independent CLI tests arrived after CLI wiring.
