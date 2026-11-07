# Memory planning trial and provisional concepts

> Execute in the existing isolated prototype branch. User approved review fixes, isolated Spark planning trial, first concept-learning slice, and a subagent to finish earlier search work.

**Goal:** Finish retrieval diagnostics, measure the runtime planning reader, and add the smallest useful evidence-backed concept lifecycle.

**Architecture:** Keep SQLite revisions/dependencies and default-off synthetic scope. Fix context contracts without changing old retrieval results. Concepts are source-backed provisional abstractions, never trusted instructions; supporting and counterexample episodes are dependency inputs with inherited permissions and forgetting. Reuse the existing goal_context evidence path. No private import, nightly migration, new database service, production deployment or automatic rule promotion.

**Spec:** docs/MECHA-MEMORY-DESIGN.md plus approved next-step description in chat.

## Tasks

- [x] Root: reproduce and fix recall-stage budget omission reporting and strict source-reference request decoding. Preserve public recall output and historical protocols.
- [x] Search subagent: additive nullable grounded_binding_count, tests for rejected/empty/unbound/non-null plans, research status addendum. No scorer/prompt/fixture changes.
- [x] Root: deploy a separately installed binary on Spark; use a small frozen synthetic runtime memory-off/on comparison with changed goals, constrained choices, source grounding and latency. Use resident model with holds; no parallel build/inference or production restart.
- [x] Concept slice: provisional named principle, explicit applicability scope, supporting episodes and counterexamples; host-validated original source spans, revision/forget/access semantics, retrieval as untrusted context. Model-generated drafts cannot select authority or invent source references. Test retrieval on a new situation and withholding after source restriction/forget. Keep learning orchestration explicit, not nightly.
- [x] Verify targeted RED→GREEN, complete required checks, one independent final review, then source-verified handoff.

## Review focus

- Recall diagnostics must count budget exclusions in initial and follow-up paths without altering retrieval ordering.
- Nested decoder strictness must be local to the new context boundary.
- Concepts must inherit all source scopes, including counterexamples, and never survive forgetting/restriction as usable ungrounded knowledge.
- Multiple excerpts from one episode must not pretend to be independent support; hypotheses remain provisional even if a model sounds certain.
- Runtime comparisons must use host-owned filters, clean synthetic fixtures and the same local model; unmeasured inference/latency remains explicitly unknown.

## Ledger

- Baseline workspace all-target build and workspace tests passed before edits.
- Search follow-up clarified: named-alternative retrieval is implemented; only report/status tail is delegated. Hybrid/vector/scaling is separate deferred work.

- Tasks 1/2 complete at `34a123b5a`: context omissions/nested reference and nullable binding counts, all targeted tests RED→GREEN.
- Concept slice complete at `b48e1e84c`: 7 core concept tests plus context retrieval/filter/budget/forget regression, 10 existing CLI integration tests and CLI parser check passed.
- Spark planning trial: 12 runs, six synthetic cases, all requested model; choices 3/6 off versus 5/6 on, choice+required-citations+visible-citations 1/6 versus 4/6 (includes one correct abstention each). Two on-arm failures used incorrect serves IDs; no retrieval occurred. Median wall time 26.5s off, 15.2s on. Diagnostic, not causal efficacy.
- Real concept smoke: 2 support episodes +1 counterexample; retained provisional; all 3 consulted sources and concept retrieved through normal goal_context, taint preserved. Plan overgeneralized and invented a deadline; integration passed, semantic quality remains unevaluated.
- Final review: no core blocking issues, one Important trial environment override. Regression observed RED; removed MECHA_PROVIDER/MODEL/EFFORT and reject successful responses with mismatched model, Python suite 2/2 GREEN. No deferred minors.
- Ruling: production/private-data qualification and migration remain deferred as approved; prototype-only deployment costs delayed real-data qualification, not a weakening of safety gates.
- Ruling: exact quotes validate provenance only; semantic entailment/generalization require a later evaluator experiment. Cost if wrong: provisional concepts can still mislead planning, as the smoke illustrates.
- Ruling: memory-off/on is an availability integration diagnostic, not matched-information/compute evidence. Cost if misread: inflated performance claims; all reporting states the distinction.
- Ruling: vector retrieval and corpus-scale indexing stay separate deferred work. Cost: unmeasured scale/semantic recall, preserving the bounded prototype scope.
- Parent owns documentation/handoff/full verification; these are being completed before handoff, not waived by review.

- Final checks: all-feature clippy passed; workspace all-feature rerun 5,968 passed / 25 ignored; standalone 62 unit + 2 CLI; Python 2/2; diff check passed. Unrelated persona fixture ENOENT passed isolated+whole rerun unchanged. Historical trial config records all 12 verified intended settings after environment-review finding.
- Handoff updated against source; Spark production remains fec8d55c7 active, separate concept binary b48e1e84c exercised with real resident model. No merge/push/private import.
- Required default-feature workspace suite also completed successfully; full logs archived with the synthetic trial artifacts.
