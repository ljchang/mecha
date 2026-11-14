# Appraisal factual support implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give situated appraisal authentic action/outcome evidence, evaluated structured statements and measured factual-support/abstention behavior.

**Architecture:** Explicit v8 files reuse existing SQLite/host authority and ordinary Agent execution. A local tool-free generator proposes typed statements; a separate bounded local pass assesses every unit and judgment; host validation and deterministic rendering admit only the allowed results. Nothing becomes Clean, policy or an accepted concept.

**Tech Stack:** Existing Rust/SQLite/serde/provider traits, Python standard-library fixture coordinator, existing static private research Site. No new dependency or service.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.8 (binding); §31.7 legacy guarantees.

## Global Constraints

- Explicit new-file v8; no live migration; preserve v1-v7 and legacy appraisal wire behavior.
- Artifact checker limit 1 MiB; complete reader artifact text at most 2,048 bytes; complete action input at most 1,024 bytes; typed omissions, never silent truncation.
- Keep 16 original passages, 16 roots, 64 source fields and atomic 20,000-byte contextual packet bound.
- At most eight units, 512 UTF-8 bytes per unit, two citations per unit/check, eight judgments, eight support IDs per judgment, 12-300 characters per quotation.
- Generation packet at most 32 KiB, evaluation packet at most 48 KiB including frame; replies/retained payload at most 16 KiB, rendered prose at most 8 KiB.
- Qualification calls at most 4,096 output tokens/180 seconds. No retries, no automatic repair. Source rechecks before and after inference; no transaction spans inference.
- Private needs/gold/host audit remain outside model inputs and reader SQLite. All consulted dependencies remain, including uncited inputs. Ordinary taint/jail/action guards remain.
- Controller owns docs, registrations, Spark Cargo writer, commits/push, PR/Site publication. Workers use assigned files only, never spawn subagents or change services.

## Review Focus

1. Valid quote plus false historical assertion: withheld by evaluated admission, with raw draft retained only in host audit; Task2.
2. True premise plus wrong goal bearing: every judgment gets its own complete check; Task2.
3. Action arguments differ from artifact bytes, or missing differs from empty: exact independently read evidence and no invented actor/cause; Task1.
4. Copied hypotheses/rejected evaluator text re-enter through descendants or prose: preserve lineage, deterministic rendering, source withdrawal; Tasks2/3.
5. Model pass fails or artifact changes between passes: no stale evaluated admission/no legacy fallback, all failed attempts recorded; Tasks2/3.

## Task 1: Authenticated bounded action/outcome capture

**Files:** modify `mecha-core/src/memory/{sql,host_capture,interpretation}.rs`, their tests and `mecha-core/src/memory_experience_bridge.rs`/tests. May add focused `memory/evidence.rs` and `memory/host_evidence_tests.rs`. Parent registers new modules if necessary. Do not edit support/evaluator/harness/docs.

**Interfaces:**
- Produce `Memory::create_supported_situated(path: &Path, domain: &str) -> Result<Memory>` with PRAGMA8 and existing tables.
- Produce `LocalAuthority::capture_with_evidence(&self, memory: &mut Memory, input: CaptureRequest<'_>, access: &Access) -> Result<CaptureReceipt>`; existing `capture` stays legacy.
- Add `PassageKind::ActionInvocation` and `PassageKind::ArtifactObservation`, source roles with same names when independent and InterpretationInfluenced when inherited; envelope remains precise about observed request/bytes.
- Existing `prepare_interpretation` accepts v8 rich captures; `retain_interpretation` rejects v8. Task2 owns evaluated retention and v8 interpretation canonicality afterwards.
- Keep preparation/source interfaces unchanged. Exact action envelope fields: `call_id`, `tool_name`, `input_status`, `input` (JSON value or null), `input_digest`; statuses Complete/Oversized. Exact artifact envelope: `content_status`, `artifact_text` (string or null), `artifact_digest`, `verdict`; statuses Complete/Missing/Refused/NonUtf8/Oversized/Unavailable. Digest/check use original full bytes. No expected rows/path fields.

- [x] Write regressions for requested-write versus actual artifact and unchanged legacy capture bytes; old code must lack requested evidence/assertion fails, not only missing symbol. Record red before implementation (controller can run core tests on Spark).
- [x] Implement explicit v8 gates and rich capture from native records/single artifact read; strict kind decoding, unique source identities/check-version, no implicit upgrades. Rich ordinary overlong passages refuse.
- [x] Test complete `[]`, empty file, missing, invalid UTF8, reader/checker overflow, duplicate JSON keys, parent/final symlinks/FIFO, changed artifact, retry conflict and v7 rich-input refusal. Assert gold/private canaries absent and current bytes/hash preserved.
- [x] Run standalone memory suite plus focused core bridge/interpretation tests via controller. Preserve complete outputs under plan workspace, report commands/results and any failing unrelated tests.
- [x] Report exact interfaces/changed files for parent commit and scoped review. Do not commit while other tasks write.

## Task 2: Typed proposals, independent checks and durable evaluated admission

**Files:** create `mecha-core/src/memory/support.rs`, `memory/support_tests.rs`, `mecha-core/src/memory_factual_support.rs`; modify `memory/interpretation.rs` and tests after Task1 releases it. May adjust shared envelope helpers only with legacy byte tests. Parent owns module registrations. No workflow/CLI/site edits.

**Interfaces:**
- `SupportProposal { units: Vec<ProposedUnit>, judgments: Vec<ProposedJudgment> }` strict serde. `ProposedUnit { id, kind, text, citations }`, kind RecordedEvent/GoalAssessment/Hypothesis/Prediction. `SourceCitation { source_id, quote }`. `ProposedJudgment { goal, bearing: InterpretationBearing, because: Vec<String> }`.
- `Memory::prepare_support(&self, prepared: &PreparedInterpretation, proposal: SupportProposal, access: &Access) -> Result<SupportRequest>` rechecks exact preparation and sources. `SupportRequest` opaque, Clone only if necessary, no Deserialize; expose `generation_input() -> &str` and diagnostic read accessors.
- `SupportReply { checks: Vec<SupportCheck> }`, each `SupportCheck { target: String, status: SupportStatus, unsupported_factual_premise: bool, citations: Vec<SourceCitation>, reason: String, questions: Vec<String> }`. Status Supported/Contradicted/Unknown. Host expected targets `unit:<id>` and `judgment:<zero-based-index>`. Reasons at most512bytes, at most4questions/256bytes each. IDs max32ASCIIalphanumeric/underscore/hyphen; judgment indices host-issued.
- `finish_support(request: SupportRequest, reply: SupportReply, evaluator_model: &str) -> Result<EvaluatedInterpretation>` validates complete checklist and creates opaque, non-Deserialize result; it never certifies arbitrary text true. Evaluator model name validated using existing identifier bounds.
- `Memory::retain_evaluated_interpretation(&mut self, prepared: PreparedInterpretation, evaluated: EvaluatedInterpretation, access: &Access) -> Result<InterpretationReceipt>` accepts only v8, checks exact binding, current authority and canonical source packet again.
- Add optional `support` metadata to `InterpretationDraft` with serde default+skipnone for v7 byte identity; v8 requires a recognized protocol and deterministic correspondence. Accepted units carry citations/kind/evaluation status; rejected entries contain only ID/status, no rejected text/reasons/questions. V7 metadata remains absent.
- Core `memory_factual_support::{generate,evaluate}` use existing Provider, GenerationBudget, CancellationToken; produce `PassOutcome<T> { result: Result<T,SupportFailure>, usage: Option<Usage>, elapsed: Duration }`. Generate takes PreparedInterpretation and returns SupportProposal; evaluate takes owned SupportRequest and returns EvaluatedInterpretation. Expose request builders for exact privacy/budget assertions. No persistence in either pass.

- [x] Write failing regressions: legacy prose false assertion can pass with valid quote; new evaluated path cannot retain it; true empty artifact plus Good bearing must be rejected/downgraded. Establish red with meaningful behavioral assertions.
- [x] Implement strict proposal/check parser and preparation; exact target coverage, source roles, source/citation membership, no unsupported model IDs/unknown JSON fields/duplicate keys; bound entire packets.
- [x] Implement independently checked per-unit AND per-judgment admission. Facts/assessments require Supported/no unsupported premise; speculative units may remain Unknown with questions but never support Good/Bad. Unknown bearing may have zero supports. Every offered judgment gets disposition/reason; never silently drop premises or clipping information.
- [x] Render from accepted units only. Keep complete raw model responses/diagnostic reasons external for caller audit; canonical reopen recomputes/validates renderer and report. Direct legacy draft, mutated evaluated reply/context/receipt, missing/unknown support protocol cannot write/read as evaluated.
- [x] Implement one bounded local call per pass, preserving served-model identity, stop/refusal/cancellation/timeout/usage semantics. Model failure never falls back to v1 or all-Supported.
- [x] Test all spec Review Focus cases, explicit Unknown and speculative premise attacks, rejected text absent from every serialized reader field, source withdrawal/forget/scopes/database replacement between passes, reopened correspondence and unchanged v7 payloads.
- [x] Run standalone plus focused core support/envelope/legacy appraisal tests through controller; report full evidence and exact public interfaces for integration. Parent commits/scoped reviewer gate.

## Task 3: Ordinary workflow and frozen fresh-case qualification

**Files:** create `mecha-core/src/memory_support_workflow.rs`, `mecha-core/examples/memory_support_trial.rs`, `prototypes/memory/support_trial.py`, `prototypes/memory/tests/test_support_trial.py`, `prototypes/memory/fixtures/support_cases.json`. Keep original situated trial/protocol untouched. Parent registers modules/examples and owns docs.

**Interfaces:** consume Tasks1/2 APIs. Use ordinary Agent::run_in/native session/FsRead/FsWrite, existing LocalAuthority/recorded_provider/LocalTrialProvider; no second executor/model transport. Explicit synthetic fresh roots, resident local model and model holds only.
- Runtime example contract: `memory_support_trial --synthetic-only NEW_ROOT ENDPOINT MODEL HOLDS_DIRECTORY SEED CASE_FILE`. CASE_FILE is one frozen case from fixtures; 64KiB regular canonical file, strict fields, no model-selected paths. Report under NEW_ROOT/report.json with all expected stages preinitialized, separate raw requests/replies, captures and evaluator dispositions. No overwrite, no auto-load/retry.
- Cases describe native input facts, requested output, actual observed output (optional), historical/current confirmed criteria and host-only annotations/probe proposals. All annotations remain out of model input. Requested and actual outputs may differ before source capture; never fabricate a native successful write for actual bytes the tool did not write.
- One generated proposal/evaluation pair plus one independently annotated probe evaluation per case. Eight fresh cases: correct, empty, missing, request/output mismatch, changed goal, unsupported causal explanation, injected instructions, unknown evidence. Preserve exact initial artifact/transcript hashes.
- Controller freezes fixture hash/protocol before model calls; use seed70510, generation/evaluation4096tokens/180seconds, case total600seconds, no retry. Each intended stage exists even on failure. Host annotation audit grades supported versus false historical claims and appropriate uncertainty, not evaluator self-scores.
- Provisional go/no-go: zero accepted false factual probe statements or wrong Good/Bad bearings; at least6/8 supported true probe statements and appropriate uncertain probes; zero manually audited unsupported factual statements in admitted generated units. All8 attempts included. Small development gate only, not general reliability.
- Pre-inference review strengthens the same gate: zero evaluator Supported checks on wrong offered bearings, and at least6/8 Supported checks on the correct offered bearing, independently of duplicate-goal admission. Bind exported outcomes to raw checks and stored support reports; preserve observed error metrics if a later stage fails. A source seal records pre-inference bytes; the controller records the exact successful build and binary/source snapshot correspondence before freeze.
- Only if gate passes, controller freezes equal-source source-only/simple-summary/evaluated-appraisal downstream comparison; no tuning reruns. If gate fails, report failures and keep concept/rumination integration behind that gate.

- [x] Write offline coordinator tests first: missing/truncated report, omitted stage, false complete status, model identity mismatch, known wrong artifact, metric denominators including failures; observe red, then implement.
- [x] Build fresh fixtures and generic native workflow with full lineage and separate observed action/output; keep source-only controls uncontaminated by probe/generator output.
- [x] Authored provider integration exercises source->richcapture->generate->evaluate->retain->ordinary GoalMemory recall->changed-goal recapture plus needs invariance, runtime cancellation and revocation. Report atomic packet sizes/withholding explicitly.
- [x] Build examples/run focused workflow tests and Python suite. Run frozen resident-model qualification after mechanical tests and scoped reviews pass. Record all attempts/costs/raw response audit; do not silently revise cases or drop failures.
- [ ] Parent updates design/ARCHITECTURE/HISTORY/HANDOFF/README/CHANGELOG, PR body and private Site with precise status/results and rumination reuse plan. Run full required checks and independent final review; push PR and monitor latest-head CI/review. No merge or production runtime switch.
