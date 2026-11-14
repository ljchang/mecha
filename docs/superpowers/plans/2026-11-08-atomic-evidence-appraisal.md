# Atomic evidence and situated appraisal

Date: 2026-11-08. Approved scope: try the smaller evaluator contract before concept learning; preserve earlier experiments and update draft PR #846 and the private research Site.

## Global constraints

- Synthetic, default-off memory prototype only. No production restart, owner-data migration, concept learning, or database migration.
- Keep v1–v3 parsing, scoring, fixtures, and frozen cohorts unchanged. Use a distinct v4 wire identity.
- Preserve original citation identity, authenticated metadata, all-source dependencies, final source reauthorization, and transactional retention. Private needs never enter model input.
- Separate three policy calls: host-numbered atomic observations, independent factual checks, current-criterion appraisal. A checked observation is not a verified world fact; semantic accuracy is measured separately.
- No generic mutation of authenticated SupportRequest. Opaque checked-fact/appraisal objects preserve original units and checks. One derived judgment may combine supported observation IDs; provisional or rejected facts cannot justify a known bearing.
- Generation emits at most eight observations, each with one original evidence handle; no model IDs or goal judgments. Unknown checks/appraisals require explicit questions. Wire schema and parser agree on every verdict.
- Comparison arms receive identical initial source evidence and criterion, resident model, seed, total output allowance (6144 tokens), and total arm deadline (540 seconds). Pipeline allocations: generation 1536, fact checking 3072, appraisal 1536. Direct reasoning gets 6144. Report actual costs and repeated input overhead; diagnostic probes are outside these budgets.
- Explicitly disable the model's thinking mode for every v4 call using the same model control in both arms; record the actual request setting. This first comparison tests bounded decomposition, not benefits from enabling additional deliberation.
- User requests parallel agents: independent implementation files in separate worktrees, parent integrates and owns shared documentation. No worker commits/pushes before required privacy checks.

### Task 1: Core receipt and atomic wire

Implement the opaque checked-facts-to-appraisal seam and bounded v4 adapters in core. Reuse quarantine execution, source catalogs and canonical retention. Expose request builders, generation, checking, appraisal and direct-baseline functions for the native workflow. Keep legacy bounds unchanged; v4 alone may use 6144 tokens and 540 seconds for direct inference. Appraisal chooses met/not_met/unknown plus supported premise IDs, reason, and question when unknown. Original atoms/checks remain unchanged. Record separate request bindings for extraction, checking and derived appraisal; never label the new judgment as independently checked against a generator-offered judgment.

Test strict no-ID generation, deterministic host IDs, exact check coverage, unknown questions, foreign/duplicate/speculative/rejected premise rejection, non-rescue of false original citations, unchanged original facts/checks, source revocation and legacy behavior. Semantic sufficiency is a model/evaluation obligation, not an invented deterministic host oracle.

### Task 2: Native paired comparison

Add a separate explicit atomic comparison runner using the existing native source/capture path and resident-only provider. Capture once before inference; export initial evidence and clone source-only controls. Alternate arm order deterministically. Preserve raw requests, responses, usage, stop failures, source identity and stage bindings. The direct arm sees only initial sources/current criterion, never pipeline outputs or annotations. Pipeline retention must reopen the stored interpretation and recall it. Direct decisions are compared as answers, without claiming independently checked retention. Source-only/historical/change-of-goal controls remain mandatory. No probes in policy inference cost. Tests cover success, missing output, schema failure, cancellation, comparison evidence identity and budget limits.

### Task 3: Independent auditor and fresh experiment

Add a separate v4 Python auditor/comparison helper without modifying old scorers. Reconstruct relationships from raw requests/responses and exported source records, not Rust-produced expected verdicts. Check exact IDs/citations/unchanged checked premises/request bindings/durable outputs; distinguish structural checks from external semantic audit. Keep failures and missing usage in denominators. Test tampered source, criterion, fact check, premise and persistence artifacts; malformed and truncated replies; legacy golden results.

Freeze implementation and auditor before independently authoring eight fresh cases and labels. Run both arms on the same resident model/evidence. Blind-review all generated factual assertions and final judgments, including observed errors from incomplete arms. Report completion, criterion accuracy, factual/citation defects, audit coverage and actual cost; no improvement claim from syntactic acceptance alone.

Predeclared semantic gate: all eight pipeline workflows and audits complete; eight current-criterion bearings correct; zero unsupported factual assertions or irrelevant original citations accepted/retained; zero wrong known bearings or unsupported derived judgments retained. Review premise sufficiency and temporal applicability separately from the final bearing label. Report generation errors that checks reject separately, without calling them retained errors. Direct-arm results use the same final-bearing and factual/citation rubric. This small gate is a pilot qualification, not proof of generalization; a tie with direct reasoning does not establish an advantage for decomposition.

### Task 4: Review, publish status and handoff

Run focused regression tests, native scripted HTTP smoke, fmt, strict all-feature Clippy, default and memory workspace suites. Review changes independently and fix important findings. Run required Spark privacy scans on exact staged changes and commit message. Update existing design, architecture, prototype docs, history/handoff, changelog, PR and private Site with actual results and limitations. Push the authorized draft branch and inspect final CI/review. Do not merge.
