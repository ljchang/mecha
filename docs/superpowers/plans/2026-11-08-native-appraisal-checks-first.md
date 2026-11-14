# Native appraisal checks-first implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Work only on assigned files in the existing isolated worktree. Parent owns integration, shared docs, PR and Site. User authorized autonomous implementation and parallel work; no additional design approval is pending.

**Goal:** Diagnose and reduce native evaluator evidence-role, verdict-consistency and uncertainty failures with one bounded repair cycle.

**Architecture:** Preserve v1 unchanged. Add an explicitly selected, read-only v2 response with evidence checks before its verdict. Host checks prove role membership and internal consistency, not semantic correctness. Compare the unchanged typed v1 arm with typed v2 on identical complete evidence through existing resident-only provider/audit/deadline machinery.

**Tech stack:** Existing Rust/serde/SQLite memory prototype; existing Spark llama-server; no new direct dependency or database migration. The standalone serde_json dependency enables its existing preserve_order feature to match core.

**Spec:** User-approved evaluator repair cycle; `docs/MECHA-MEMORY-DESIGN.md` §31.14 and `docs/ARCHITECTURE.md` Experimental memory prototype. This plan specifies the bounded optional extension.

## Global constraints

- No production config, service restart, owner data, graph migration, nightly jobs or merge.
- Preserve legacy input/response meaning and prior experiment artifacts. New request identity binds v2; cross-protocol replay fails.
- Recheck complete authorization closure before/after inference, including uncited/omitted sources; read-only and no transaction across inference.
- Host withholding is not successful model reasoning. Preserve raw response and count calls even on parser/consistency refusal.
- Use Qwen3.6-35b-a3b-uncensored if still resident, seed70510, temperature0.6, thinking off, max6144 output tokens, one call per arm, independent540-second deadlines. No builds during inference.
- Existing standalone memory harness must compile without provider dependencies.

## Review focus

- Correct-looking JSON in ordinary text cannot impersonate authenticated observations.
- Requested writes/receipts cannot satisfy an artifact-content check; genuinely empty complete content is distinct from unavailable content.
- An execution-only criterion can legitimately leave artifact content unassessed; do not introduce a universal artifact gate.
- Parsed comparison and verdict must agree; source/criterion identity and full source closure still bind rejected and unknown responses.
- Actual schema property order must put state last; compile the exact schema with the pinned engine, not only a schema validator.

### Task 1: Opt-in checked native response

**Files:** `mecha-core/src/memory/native_evidence.rs`, new `native_appraisal_checks.rs` child module and tests if useful. Avoid workflow/example edits.

**Interfaces:** Preserve current API. Add `NativeEvidenceRequest::checked_input(format)`, `checked_generation_request(model,budget,format)` and `Memory::finish_native_checked_appraisal(request,response,access)`. Return a distinct serializable `NativeCheckedAppraisal` containing v2 request_key, artifact_content, artifact_facts, comparison, because, reason, questions, state. Expose fields/types through current interpretation reexports.

- [x] Add meaningful failing tests before implementation; baseline must pass before product edits.
- [x] Use one flat closed schema, properties in order: request_key, artifact_content, artifact_facts, comparison, because, reason, questions, state. No oneOf. `artifact_content`: not_assessed|complete|unavailable; `comparison`: supports|contradicts|insufficient; verdict retains met|not_met|unknown. Required fields all present. Request key binds v2 domain plus original request identity; fact IDs stay unchanged.
- [x] Model prompt explicitly judges current criterion, uses only observed artifact text for observed contents, treats invocation as intention and historical checks as history, distinguishes unknown from disproven and observation from causal identification. Prior content stays untrusted data. Reasons max512 characters/2048 UTF8 bytes, questions max4 of64 characters/256 bytes, response max8192 bytes, IDs max8 unique. Generation input+schema remains bounded128KiB.
- [x] Host validates artifact_facts as a subset of because, issued IDs only: complete requires at least one selected authenticated ArtifactObservation with Complete+Some text, every selected artifact complete; unavailable permits only noncomplete/absent-text ArtifactObservation IDs, or empty only if no complete observation exists; not_assessed requires empty artifact_facts. Ordinary text, invocation and receipt IDs are never artifact facts. Artifact assessment is optional because criteria can concern execution rather than content.
- [x] Compare enum consistency: supports=>met; contradicts=>not_met; insufficient=>unknown. Known requires nonempty because and no questions; unknown requires questions. Invalid consistency is refused with original output preserved by workflow. No prose entailment claim.
- [x] Test all enum mappings, wrong evidence roles/status, empty versus unavailable, source withdrawal/staleness/uncited dependency change, v1/v2 replay, duplicate keys/IDs, bounds, state-last order and read-only behavior. Run focused and standalone checks, report changes without committing shared work.

### Task 2: Diagnostic workflow and controls

**Files:** `mecha-core/src/memory_native_workflow.rs`, `mecha-core/examples/memory_native_trial.rs`, workflow tests. Coordinate Task1 API names; avoid Task1 files.

- [x] Preserve `run_case` and original CLI. Add `run_checked_case` paired direct/checked typed arms with explicit report protocol; example accepts a trailing `--checks-first` option. Raw/typed v1 path unchanged.
- [x] Reuse source preparation, complete auth rechecks, immutable source/database checks, recorded provider, per-arm independent deadlines, model identity, cancellation and call accounting. Retain raw model reply and structured refusal separately. Do not silently map a refusal into Unknown or repair outputs.
- [x] Add a separately selected `--verdict-controls` mode using same preparation, effective schemas and LocalTrialProvider. Exactly three copy controls per protocol (six total) explicitly request a literal schema-valid response for met/not_met/unknown. These controls measure ability to emit verdicts, not evidence reasoning. Include suitable issued IDs/status and mandatory unknown question; no fixture expected labels used by semantic trials. Save target, effective request, output, checks and accounting under separately labeled controls protocol.
- [x] Tests: legacy interface compatibility, intended arm names/order, direct/checked distinct schemas and request identities, unknown/consistency rejection accounting, both arms after independent failure, source withdrawal before/after dispatch and no inference after revoked closure. Record actual HTTP schema field order using existing scripted-provider mechanisms.

### Task 3: Qualification and one fixed comparison

- [x] Clean baseline workspace/all-target build and default tests on Spark; parent begins edits only after baseline passes.
- [x] Independent code review; required fmt, strict Clippy, workspace/default+memory tests, standalone harness and appropriate existing Python actual-binary checks; fix regressions before inference.
- [x] Freeze exact implementation/binary before model outputs. Six controls, eight reused-case direct/checked pairs and four new independently authored pairs = at most30 calls. Fresh cases authored before outputs; align goal_text with criterion_text to avoid known stale-goal ambiguity. No prompt/fixture tuning within cohort, no silent retries. Record prepared source IDs differ by capture but identical within pair.
- [x] Independent blinded audit against frozen expected states, sufficient premises and complete faithful explanations. Separate raw state accuracy, host acceptance/withholding, semantic pass, abstention and cost. Do not attribute a bundled protocol improvement only to field order.
- [x] If six controls cannot emit distinct intended labels, stop semantic comparison and diagnose transport/schema instead. If independent source safety checks fail, stop. Model-quality failure is a result, not reason for unlimited retries.
- Publication gate: source-verified handoff/HISTORY/architecture/design/CHANGELOG and PR updates, strict Spark privacy scan before commit/push, CI/automatic review loop, and private Site publication. Exact completion is recorded on PR #846 after this pre-publication plan is committed. No merge. Concept-benefit pilot remains the next separate experiment, using independent synthetic outcome scoring.

**Measured outcome:** six controls passed; 24 semantic responses completed. Direct/checked primary semantic passes 5/12 and 4/12; accepted semantic passes 5/12 and 2/12; unknown 0/4 each. Preserve the negative result and proceed only with independently scored synthetic concept experiments. See HISTORY for the full audit and compatibility limits.
