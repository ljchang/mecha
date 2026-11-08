# Factual Support Wire and Reasoning Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Execute the authorized work without repeated permission checks.

**Goal:** Repair model citation/schema compliance and independently test evaluator reasoning before concept learning.
**Architecture:** Explicit wire-v2 adapters expand host-issued citations into existing canonical v8 types. Existing quarantined calls, native workflow and store retain authority. One current-criterion assessment plus per-judgment premise sufficiency yields coherent bearing checks.
**Tech Stack:** Existing Rust/serde/SQLite/provider traits and Python offline qualification; no new dependency or service.
**Spec:** docs/MECHA-MEMORY-DESIGN.md section31.10 (section31.8 remains canonical admission contract).

## Global Constraints

- Preserve legacy wire-v1 paths, existing v8 records/canonical reopen, all source authority/revocation, local-only/no-autoload/holds/cancellation/refusal/identity guards. No migration, merge, production restart, nightly migration or concept implementation.
- Citation quotes remain 12–300 Unicode scalars, <=2 per canonical unit/check. All visible source chunks eligible; no gold-driven selection. All original sources remain supplied, with roles and exact identity. Whole-packet overflow refuses.
- Strict duplicate/unknown-key deserialization, request binding, complete target coverage and existing canonical validation remain authoritative despite constrained output. Raw and expanded replies/proposals <=16KiB; unit text/reason <=512 UTF-8 bytes; <=4 questions of <=256 bytes; <=2 exact citations. Schema character limits never replace host byte checks.
- Existing budgets:4096 output tokens,180s call,600s case,3 calls/case,no retries. Complete request including frame/schema <=32KiB generator and48KiB evaluator.
- Parent owns Spark Cargo/model writes, lib.rs registrations, integration, shared HANDOFF/HISTORY/design docs, PR and Site. Workers never run Cargo/model calls without parent coordination; never commit shared files.
- All original trial artifacts remain unchanged. Development rounds and fresh qualification are separate, all attempts/costs retained. No claim of reasoning repair from schema completion alone.

## Review Focus

- Identical text in different revisions, cross-request handles, Unicode boundaries and uncitable short sources retain exact identity or refuse.
- A global Met result cannot promote a judgment with missing/irrelevant/rejected/speculative premises; both Good and Bad cannot be supported together.
- Historical success/current failure or unknown and requested/observed/missing artifacts stay distinct; checker gold stays host-only.
- Unsupported historical facts in predictions and approval laundering remain tested semantically, beyond shape checks.
- Actual wire response_format is audited; a provider that cannot constrain output refuses; no post-hoc repair or silent legacy fallback.

## Task 1: Citation catalog and constrained generation

Files:create mecha-core/src/memory_support_wire.rs (inline tests). Parent registers it.
Interfaces:pub(crate) CitationCatalog::new(prepared:&PreparedInterpretation,phase:&str,binding_input:&str)->Result<Self>, request_key()->&str, input()->Value, handles()->Vec<String>, resolve(&[String])->Result<Vec<SourceCitation>>. Private catalog owns exact binding and original identities. generation_input(prepared)->Result<(String,Value)> returns complete input and JSON schema; parse_generation(prepared,text)->Result<SupportProposal>. FRAME constant supplies generic generation instructions.
- [ ] Write failing citation reconstruction, Unicode/boundary, cross-request/foreign/duplicate handles, strict wire shape and wrong goal/prepared-context tests.
- [ ] Implement deterministic balanced chunks, complete visibility, request key derived from opaque prepared request identity, phase discriminator, and exact binding input (unambiguous length-delimited hash), finite handle enums; no invented quote or source.
- [ ] Wire DTO requires request_key, units(id/kind/text/evidence handles), judgments(bearing/because). Host supplies exact current goal. Strictly decode then unchanged validate_support_proposal.
- [ ] Test legacy/canonical compatibility and complete packet bounds via parent; independent task review.

## Task 2: Evidence-first evaluator adapter

Files:create mecha-core/src/memory_support_reasoning.rs (inline tests). Consume CitationCatalog above; parent registers.
Interfaces:evaluation_input(request:&SupportRequest)->Result<(String,Value)>; parse_evaluation(request:&SupportRequest,text:&str)->Result<SupportReply>; FRAME. Bind request_key to opaque prepared request identity plus evaluation phase and request.generation_input(), including exact proposal/targets. Parent adds crate-private PreparedInterpretation::request_identity() getter; no private needs values enter prompts.
- [ ] Write failing opposite-bearing coherence and irrelevant-premise tests plus temporal, missing/empty, speculation and unknown cases using authored replies.
- [ ] Strict wire includes bounded criterion comparison with Met/NotMet/Unknown, one check per unit with premise state None/Supported/Contradicted/Unknown, and one per offered judgment with premise sufficiency and diagnostics. Cite catalog handles. Return existing canonical SupportReply only after exact coverage and structural validation.
- [ ] Map factual premise Unknown/Contradicted to unsupported=true; factual units cannot use None. Derive offered-bearing status consistently, without promoting missing/insufficient premises. Existing finish_support remains final authority.
- [ ] Exact mapping: Met+Good or NotMet+Bad is Supported only when premises_sufficient=true; matching but insufficient is Unknown. Opposite known bearing is Contradicted. Unknown criterion+known bearing is Unknown. Offered Unknown is Supported only for Unknown criterion, otherwise Contradicted. Every known offered judgment with empty because must have premises_sufficient=false. Unit acceptance remains an additional host gate after this semantic sufficiency check.
- [ ] Each wire criterion, unit and judgment has its own bounded citations/reason/questions. Canonical unit/judgment check copies that target's fields exactly; never union/truncate criterion+target citations or synthesize missing questions. Supported requires 1–2 citations and no questions, Unknown 1–4 questions; reject inconsistent combinations. Criterion Unknown separately requires missing-data questions, known criterion requires citations/no questions. Thus a Supported offered-Unknown judgment has no questions while the separate criterion diagnostic retains the missing data. Unit Supported with factual-premises Unknown/Contradicted refuses as inconsistent; factual units with None refuse. No model reasoning field is discarded from host audit.
- [ ] Evidence-first frame distinguishes observations/attribution, historical/current criteria and requested/observed outputs. No dataset/domain switch or hidden expected rows.
- [ ] Parent runs tests; independent task review.

## Task 3: Provider and workflow integration

Files:memory_factual_support.rs,memory_trial_provider.rs/tests,memory_support_workflow.rs,example memory_support_trial.rs,lib.rs; controller may delegate disjoint implementation.
- [ ] Add explicit structured local provider constructor with existing LlamaJson mode and Provider capability delegation; legacy new unchanged. Authored HTTP tests prove exact constrained wire and refusal/cancel/identity behavior.
- [ ] Add generate_grounded/evaluate_grounded using shared pass envelope, wire adapters and response_schema. Count full input/frame/schema against limits. No repair/retry/fallback.
- [ ] Explicit workflow protocol entrypoint and example flag preserve old default behavior. Audit both raw wire and canonical outcomes; qualification scorer validates correspondence by protocol.
- [ ] Authored native end-to-end test exercises generate/evaluate/admit/recall and source withdrawal with wire-v2; legacy tests remain green.

## Task 4: Independent qualification and publication

Files:prototypes/memory/support_trial.py,tests/test_support_trial.py,new independent fixture;docs/HANDOFF,HISTORY,ARCHITECTURE,README,CHANGELOG and private Site parent-owned.
- [ ] Extend shared coordinator for explicit wire-v2 and externally selected frozen fixture without changing wire-v1 manifests/scoring. No duplicated transport/scorer. Verify exact schema in recorded effective body and raw-to-canonical translation. Wire-v2 false-Supported metrics count raw canonical check.status == Supported even when an unsupported-premise flag prevents admission; keep this distinct from false admission and retain legacy scoring separately.
- [ ] Independently author eight fresh cases, balance known/unknown outcomes and shuffle offered order; keep annotations out of model inputs. Leave fixtures unread by implementers until freeze.
- [ ] Mechanical gates and review, then <=2 separately recorded development rounds on old cases (not new evidence). Freeze fresh inputs/build/hash before qualification; audit all actual admissions independently.
- [ ] Run required build/fmt/Clippy/workspace+memory checks and scoped regressions, privacy scan; update PR and Site with exact status and observe latest-head CI/review. No automatic concept phase on failed support gate.

## Development findings and bounded second refinement (2026-11-08)

The first reused-case development run completed five native workflows out of
eight, with all 24 calls retained. Its original coordinator ledger remains
failed: the auditor incorrectly looked for the no-autoload policy in the JSON
body, while the provider enforces it in the URL query. HTTP tests verify that
policy; corrected offline diagnostics are recorded separately, not substituted
for the frozen run.

The next refinement is limited to role-prefixed request-bound citation handles,
128-character producer text/reason limits and 64-character questions (canonical
512/256 UTF-8 byte checks remain), a criterion schema with separate complete
known/unknown branches, and generic clarification that the top-level current
criterion is authoritative and speculative wording cannot authenticate factual
antecedents. These changes address observed failure classes without adding
case-specific rules or changing canonical storage.

The v2 scorecard must also count wrong generated Supported/retained bearings,
including dropped duplicate judgments and independently available partial
outcomes. Probe correctness cannot substitute for generated interpretation
quality. A separately authored replacement holdout remains unread by the
implementers and will be frozen after development. The first draft is unused
because its design report was accidentally opened before the controller freeze.

## Final structural correction after development (2026-11-08)

The second and final reused-case round completed 5/8 workflows. Two evaluators
selected `None` for a factual unit's premises; another returned Unknown without
a missing-information question. The host correctly refused all three. Final
producer schemas therefore partition unit targets by factual/speculative kind
and Supported/Contradicted/Unknown status. Supported requires citations, no
questions and supported factual premises; Unknown requires questions; factual
targets never permit None. These are existing parser constraints, not new
semantic judgments. Empty target arrays remain explicit. No third development
replay or further semantic prompt tuning precedes the fresh freeze.

## Post-qualification citation correction

The frozen fresh cohort at source `05f47d317` completed 6/8 workflows. A matching
Unknown judgment had no citation and therefore failed the canonical Supported
diagnostic. After preserving that assessment, a separate producer-only change
requires at least one examined-source citation for every judgment. The host's
Unknown/Contradicted parser behavior is unchanged, and this small correction has
no fresh-model quality claim. The cohort is not replayed or reclassified. Current
artifact/criterion reasoning still fails independently, so concepts stay deferred.
