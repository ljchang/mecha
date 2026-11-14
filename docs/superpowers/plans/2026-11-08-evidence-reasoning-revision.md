# Evidence reasoning and judgment revision implementation plan

> **For agentic workers:** Use subagent-driven-development with isolated worktrees, regression-first implementation, scoped reviews and one final integrated review.

**Goal:** Repair original-citation evaluation, expose authenticated capture context, and allow a later qualified judgment to replace an earlier Unknown without rewriting history.

**Architecture:** Preserve canonical SQLite v8 sources and legacy producer/report semantics. Add request-local evidence metadata and explicit wire-v3 generation/evaluation over the existing provider. Add a versioned support-report selection policy; model semantic judgments remain fallible.

**Tech Stack:** Existing Rust/serde/rusqlite, Python qualification harness, existing private static Site.

**Spec:** docs/MECHA-MEMORY-DESIGN.md §31.11 (this increment; previous §31.10 remains historical).

## Global constraints

- No production restart, owner-store migration, nightly changes, concept learning, or rumination implementation. No merge.
- Preserve v1/v2 adapters and old v8 report reconstruction exactly. No implicit database upgrade.
- No model name dispatch in agent.rs, no new transport, no additional model pass or retry.
- Preserve source authority, dependency closure, taint, revocation, strict JSON keys, canonical UTF-8 limits and existing 32/48 KiB complete request budgets.
- Host metadata comes from authenticated capture records; missing times stay unknown. Selected capture does not mean globally newest. Never infer resource identity from matching text or opaque IDs.
- Retain all offered judgments and original citations in audit. Model assessments do not grant action permission or establish ground truth.
- Frozen previous model artifacts remain immutable. No tuning or reruns of the former fresh cohort. Any new qualification uses independently authored fresh cases after freeze, all attempts counted.
- Parent owns integration, docs, Sites, remote Cargo/model operations and PR writes. Independent implementations use separate worktrees. No worker subagents.

## Review focus

1. Reused identical source text across captures must not collapse provenance or leak inaccessible captures.
2. A valid replacement citation must not conceal an irrelevant ORIGINAL unit citation.
3. Absent evidence of causation must not be treated as evidence against causation; uncertainty statements may themselves be supported.
4. A later Supported judgment with one rejected/speculative premise must not replace Unknown.
5. Historical v8 drafts must reopen identically; conflicting eligible Good/Bad judgments must remain Unknown.

## Task 1: Authenticated request-local evidence metadata

Files: memory/interpretation.rs; new memory/evidence_metadata.rs and focused tests.

- Keep SourceField and legacy generation_input serialization unchanged.
- Add a private metadata sidecar to PreparedInterpretation and public read-only accessor evidence_metadata().
- Public serializable EvidenceMetadata: source_id, role, interpretation_influenced, captures.
- EvidenceRole: HistoricalCriterion, ExecutionReceipt, ArtifactCheck, UserStatement, AssistantStatement, ToolResult, ActionInvocation, ArtifactObservation, PriorInterpretation, Unclassified.
- CaptureAssociation: capture:RevisionRef, relation: SelectedCapture|Dependency, capture_time:Option<String>, passage_time:Option<String>, transcript_line:Option<u64>.
- Build metadata from canonical projections already in the authorized source closure. Preserve every matching association from those projections, including multiple captures; never query unrelated captures to enrich metadata. Prior interpretations can have no capture association. An otherwise valid original-only prior root with no projection in the authorized closure gets Unclassified and empty associations rather than breaking legacy preparation or querying unrelated captures. Do not expose new raw transcript/session information.
- Projection /criterion,/receipts,/artifact have distinct roles. For original passages use authenticated kind even when SourceKind is InterpretationInfluenced; retain influence as orthogonal boolean.
- Independently compare the sidecar in support preparation rechecks, supplied-versus-evaluated preparation matching and final retention reconstruction; unchanged legacy input hashes alone are insufficient. Canonical timestamps and transcript line are descriptive only; no chronology across captures inferred.
- Regression tests: selected vs dependency, missing timestamps, identical text distinct sources, shared originals multiple eligible capture associations, influenced artifact role survives, no unauthorized/unconsulted capture metadata; legacy generation bytes unchanged.
- Run standalone prototype tests for RED/GREEN; do not run broad concurrent Cargo builds. Return diff and test evidence, no commit/push before parent privacy gate.

## Task 2: Versioned candidate judgment revision

Files: memory/support.rs and support_tests.rs.

- Add SUPPORT_REVISION_PROTOCOL="situated-factual-support-v2" and SUPPORT_REVISION_PRODUCER="situated-interpretation-v3".
- Add finish_support_revised(request,reply,evaluator_model) sharing existing unit admission. Existing finish_support stays byte-compatible.
- Assess all judgments before selection using unchanged own-check/all-original-factual-premise gates.
- If eligible known bearings disagree, render one Unknown and retain explicit conflict dispositions. Otherwise choose the FIRST eligible known judgment (stable tie-break for same-bearing duplicates). Earlier Unknown is superseded. If no known qualifies choose first offered conservative Unknown.
- Preserve full dispositions, indices and original because lists. Add optional unsupported_factual_premise:Option<bool> to JudgmentDisposition, omitted for legacy and required for the new protocol; reconstruct eligibility from this flag/status/premises, never from the reason being validated. Add Superseded and Conflicting reason variants only for new protocol; old validation rejects these.
- New validator derives and verifies selection from full report, enforces at most one retained known bearing and detects forged reason/bearing/index. Canonical renderer dispatches by report protocol and emits matching producer version.
- retain_evaluated_interpretation reconstructs via protocol dispatch, never by silently upgrading a legacy report. Exact retry conflicts, provenance and all admission gates remain.
- Tests: downgraded/explicit Unknown followed by qualified known; rejected/speculative/missing/partial premise does not qualify; conflicting knowns -> Unknown; same-bearing stable selection; all disposition audit; tamper/reopen/legacy compatibility and retry behavior; Supported own-check with unsupported_factual_premise=true cannot qualify despite otherwise accepted premises.
- This fixes same-request selection only. Separate historical interpretation records are preserved, not auto-withdrawn or relabeled current.

## Task 3: Evidence-aware wire-v3 and native integration

Files: memory_support_wire.rs, memory_support_reasoning.rs, optional focused memory_support_evidence.rs, memory_factual_support.rs, memory_support_workflow.rs and example.

- Reuse catalog chunking and existing diagnostics/schema construction; avoid a copied v2 module. Add explicit v3 entrypoints and SupportProtocol::EvidenceV3 / --evidence-v3; old entrypoints unchanged.
- Include complete Task1 metadata alongside sources in generation and evaluation packets; bind it into phase-specific request keys. Full packet/frame/schema count against unchanged budgets.
- Keep generator canonical proposal shape. Explicitly require factual units to cite the sources establishing the statement; current criterion is context, not a fabricated source citation.
- Evaluator v3 unit DTO adds original_citations:[{citation_index,relation}] with relation DirectSupport|ContextOnly|Contradiction|Uncertain. Exactly one assessment for each ORIGINAL canonical citation slot, no duplicates/missing/foreign indices. Supplied source quotations/roles remain visible.
- Reuse bounded evaluator evidence handles as optional counterevidence/diagnostic context; they cannot replace original support. For Supported units construct canonical SupportCheck.citations from the original DirectSupport slots, not evaluator evidence; diagnostic evidence may be empty and remains only in raw host audit. Supported units require at least one original DirectSupport and all original citations DirectSupport (strict retention: context-only citations should be omitted/repaired by a new proposal, not retained as claim evidence). Factual premises must satisfy unchanged rules. No-citation pure hypotheses may retain Unknown only.
- Any original Contradiction prevents Supported unit status. Factual-premise Unknown remains valid and never automatically becomes Contradicted. If an original citation contradicts an Unknown speculative unit, mark it unsupported for admission so it cannot survive as ordinary uncertainty; test this explicitly. Semantic assertions are still model judgments, not host entailment proofs.
- Criterion diagnostic explicitly compares current requirement with selected-capture observations, distinguishes historical action/receipt/check, changed goals, missing artifacts, absent causal experiments and valid conditional/uncertainty statements. Do not hardcode Missing -> NotMet or latest-source-wins.
- Preserve v2 criterion derivation and judgment premise checks; v3 completion uses Task2 revised selection.
- Tests: correct replacement evidence cannot rescue wrong original citation; exact original-slot coverage; contradictory labels rejected; changed metadata changes request key; legacy schema/output unaffected; authored native workflow retains later supported judgment; revocation, budget and provider controls preserved.

## Task 4: Independent qualification and reporting

Files: prototypes/memory/support_trial.py, tests, new independent fixtures; docs; existing private Site.

- Extend coordinator/auditor for explicit v3 while preserving v1/v2 behavior. Independently reconstruct metadata, exact original-slot coverage, canonical selection/disposition and wire consistency.
- Score evaluator support, actual retained bearing and original-citation relevance separately; no clean-citation claim from zero false facts. Require audit coverage and zero citation defects for new gate.
- Preserve generation failures and partial probes as unknown in fixed denominators. Existing call budgets 4096 output / 180 seconds per call / 600 seconds per case / at most3 calls remain.
- Fresh model test only if Spark reachable; never load/swap model. Freeze source,binary,helper,fixture,model before inference. One bounded eight-case qualification, no within-attempt retries or edits. If unavailable, report model benefit unmeasured and keep gate closed.
- Full build/fmt/Clippy/default and memory suites, meaningful RED/GREEN regressions, independent scoped/final review, required Spark privacy scan before commits/push. Do not bypass unavailable privacy scan.
- Keep PR #846 draft and private Site current, with implemented/mechanically verified/model-qualified boundaries. Observe latest-head CI and automated review to completion; fix confirmed major/medium findings.
