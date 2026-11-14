# Situated interpretation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist source-grounded, context-bound interpretations and use them as untrusted hypotheses in ordinary planning while preserving original episodes.

**Architecture:** A shared core reply/grounding engine supports both the unchanged legacy graph follow-up and a new bounded v2 producer. Explicit v7 memory admission owns durable dependencies and context-specific eligibility. Private needs are assessed on the host, independently of model text and action permission.

**Tech Stack:** Existing Rust workspace, rusqlite/SQLite, serde, existing Provider/QuarantinedPass and GoalMemory; synthetic fixtures on Spark.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31, especially §31.7.

## Global Constraints

- No new dependencies, service, config switch, executor, production activation, owner-graph migration or nightly writer.
- Keep graph request/body bytes and legacy Clean-only consumers unchanged.
- New files only for v7; retain v1-v6 compatibility and refusal behavior.
- All consulted inputs are dependencies; preparation/admission and selection/return independently check current authority.
- Max 16 source roots, 64 fields, 32 KiB packet, 16 claims, 12-300-character quotes, 2,000-character statements, 8 KiB prose, 16 KiB reply.
- Explicit generation budget <=8,192 tokens and <=600 seconds; demonstration 4,096/180.
- At most two contextual interpretations, complete dependency bundles within the existing 20,000-byte tool packet.
- Raw sensors, thresholds, sensor rationale and low-entropy sensor hashes remain host-only. No automatic scalar utility or permission changes.
- Baseline629f787db already passed full Spark build, default6,326/memory6,695 tests, strict Clippy, Python101+16; use that verified unchanged baseline.

## Review Focus

- Duplicate local tool IDs across episodes must not cross-ground a citation (Tasks1/2).
- An uncited consulted source withdrawn during inference or final return must invalidate the result (Task1).
- Contextual interpretation read followed by capture must retain complete original lineage (Tasks1/3).
- A recaptured descendant reinterpreted for a new goal labels old hypotheses HistoricalDependency, never SelectedCurrent; default readers withhold the whole interpretation-bearing bundle (Tasks1/3).
- Needs-only changes must alter host assessment without changing model request or leaking private canaries (Task3).
- Legacy parser, graph prefix, default readers and standalone memory crate remain compatible (Tasks1/2).

### Task 1: v7 guarded interpretation storage and reader

**Files:** Create `mecha-core/src/memory/interpretation.rs`, `interpretation_tests.rs`; modify `memory/{mod,model,sql,store,host_capture,context,forget}.rs` and exhaustive Record matches as necessary. No tool/core-provider edits.

**Interfaces:** Define in `memory::interpretation`:
- `InterpretationContext::new(ConfirmedCriterion, host_context_id: String) -> Result<Self>` and `read_policy() -> InterpretationRead`; private fields, no Deserialize.
- `PreparedInterpretation` private fields; read-only getters for context, qualified `SourceField`s and exact generation input; no Deserialize.
- `SourceField` has `id`, `source: RevisionRef`, `field`, `text` and explicit source kind; IDs host-minted and collision-free.
- Typed `InterpretationDraft` contains prose, grounded claims (`statement`, qualified source ID, quote), current-goal judgments/support indices, prediction, grounding counts and model/version. Final exact DTO fields are frozen in the implementation brief before Task2's adapter.
- `Memory::{create_situated, prepare_interpretation, retain_interpretation, withdraw_interpretation, context_with_interpretations}` following §31.7 and audit interface sketch. Prepare takes real CaptureReceipt, prior refs, context, snapshot and Access; retain consumes private prepared object plus typed draft and Access; contextual read takes existing ContextRequest/ContextPolicy/Situation plus InterpretationRead.
- `Record::Interpretation` carries only bounded untrusted hypothesis data and opaque host context reference, no sensor assessment.

- [ ] Add executable red tests for new schema/admission/read behavior, recording expected missing-feature failure; preserve v6 baseline tests.
- [ ] Implement explicit v7 creation/schema validation, private prepared receipts, independent source/quote/scope admission, idempotent persistence and dependency-safe forgetting/withdrawal.
- [ ] Implement context-key reader with complete bundles and independent final checks; exclude interpretations from default search/recall/context.
- [ ] Permit only canonical v7 interpretations as recapture dependencies, retaining transitive bridge authority; legacy producers remain conservative.
- [ ] Run focused core/standalone tests, fmt and relevant compatibility tests; report source files and red/green evidence for scoped review. Parent owns commits.

### Task 2: shared generation, parsing and grounding

**Files:** Create `mecha-core/src/interpretation.rs`, tests as needed; modify `distill.rs`, `appraisal_store.rs`, `lib.rs`. Later create `memory_situated_appraisal.rs` for v2 producer; coordinate with Task3 on its host-needs submodule. No memory storage/tool edits.

**Interfaces:** Shared crate-private `ReplyClaim { statement, pointer: String, quote }`, `ReplyDraft` (legacy draft semantics with raw pointers), `ReplyTurn { draft, usage, elapsed, stop_reason }`; `parse_reply(&str) -> Result<ReplyDraft, distill::Malformed>`; `complete(&dyn Provider, &CompletionRequest) -> anyhow::Result<ReplyTurn>`.
Shared `admit_claim_indices(offered, max_claims, check)` returns retained indices and Grounding; `remap_support` preserves old order, dropped support and duplicate clipping. Legacy adapters resolve Pointer exactly as before. V2 resolves only prepared SourceField IDs through existing grounding::admit and constructs Task1 DTO; no Pointer::Unread reuse.
The v2 producer has explicit budget/deadline/cancellation, one quarantined tool-free call, model-independent context preparation, and returns typed diagnostic/outcome rather than partially storing a malformed draft.

- [ ] Freeze existing parser, refusal/truncation, graph frame/body and prefix behavior with compatibility tests; add red tests for graph-independent qualified source grounding and whole-packet limits.
- [ ] Extract shared parser/provider envelope and batch support mapping; route legacy adapters through it without changing wire bytes or historical store semantics.
- [ ] Implement bounded v2 request and producer, prepared-source grounding, unknown-goal refusal/counts and qualified citations; persist only via Task1 admission after inference.
- [ ] Test malformed/refused/truncated (including valid JSON at MaxTokens)/timeout/cancel paths, no tools/history, two-episode ID collisions, false quotes and unsupported judgments. Run focused tests and legacy suites; scoped review before parent commits.

### Task 3: host needs, ordinary planning and connected qualification

**Files:** `memory_situated_appraisal/needs.rs` (or equivalently focused core module), `tool/goal_memory.rs` and tests; new `mecha-core/examples/memory_situated_appraisal.rs`; bounded comparison under `prototypes/memory/`; parent-owned docs/Site.

**Interfaces:** Reuse reading::Reading and LineReading; non-serializable host input with observation time and explicit freshness threshold. Owner-audit result keeps missing/stale/withdrawn/unread/deferred/sparse/nothing/observed distinct. Generation never receives this payload. `GoalMemory::with_interpretations(InterpretationRead) -> Result<Self>` opts in after verifying its confirmed goal matches; ordinary constructors unchanged. Use Task1 contextual reader, same taint and no new tool arguments.

- [ ] Add red tests for needs-only request invariance, context mismatch, bounded untrusted planning reads and read-then-capture lineage.
- [ ] Implement host needs assessment and tool adapter; refuse invalid/future freshness inputs; preserve active withdrawn distinctions without goal reprioritization.
- [ ] Build authored connected example with one native captured episode, two confirmed goals, reopened store, ordinary planning tool, separately evaluated artifact and subsequent capture. Source bytes stay unchanged; no self-graded semantic claim.
- [ ] Freeze a small synthetic model protocol before calls: same evidence/two goals, explicit current criterion, hidden independent goal-specific rubric, no retries, all failed slots retained; compare source-only against interpretation-assisted planning with costs stated. Inspect resident router, never load/switch model, protect gold/private needs from wire input.
- [ ] Run required full Spark workspace build/default+memory suites/fmt/strict Clippy/standalone+Python checks once final source is frozen, owner-data privacy scan, independent final review; fix major/medium findings.
- [ ] Update HANDOFF/HISTORY/README/CHANGELOG, PR846 and private living Site with actual capability/evidence boundaries. Push once qualified, follow final GitHub review/checks, retain draft/no merge.

## Self-review

The new reader is an explicit untrusted hypothesis channel, not a Clean exception. No sensor behavior depends on hidden model information. Source normalization lives in memory; legacy and v2 grounding adapters share core algorithms but storage independently validates its boundary. The standalone crate remains provider/core-independent. Capture reliability repairs and learned concepts/rumination are tracked separately, not hidden prerequisites to this increment.
