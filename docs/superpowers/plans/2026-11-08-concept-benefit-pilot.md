# Concept benefit and revision pilot implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Check off tasks only after independent review and qualification.

**Goal:** Measure one provisional concept and one counterexample-driven revision on six untouched decisions with equal original evidence in both arms.

**Architecture:** Reuse the existing concept storage/admission and rumination APIs. A feature-gated core workflow makes at most fourteen audited model calls through the existing resident-only transport; a separate Python scorer computes fixture-defined outcomes without inference. A thin example is the executable entry point.

**Tech stack:** Existing Rust/SQLite/serde/provider infrastructure and Python standard library. No new dependency or database schema.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.15. This implements the already approved bounded pilot; production learning and native-source admission remain outside scope.

## Global constraints

- Synthetic, unbound original captures only; two training episodes and one counterexample. No owner data, production activation, learned-rule installation, nightly scheduling, migration or merge.
- Fourteen planned calls: induction, three equal-evidence decision pairs, rumination, three new pairs. One first response per slot, no transport retry or resume, fresh output root only.
- Each phase has applicable structure, near-match exception and insufficient-evidence cases. Both decision arms receive identical current facts and original episodes; only the provisional concept slot differs. Counterexample is supplied to both post-revision arms. Never put future decisions in induction or rumination.
- Existing selected model qwen3.6-35b-a3b-uncensored; seed 70510, temperature 0.6, thinking disabled, 6144 output tokens, independent 540-second deadlines including discovery. Preserve the existing 16 KiB transport ceiling; never truncate replies into admissible drafts.
- Concept induction failure withholds dependent stages. Rumination failure withholds the post phase. Unchanged/abstained rumination permits explicitly labeled unchanged-concept follow-up, never a revision-benefit claim. All planned slots remain in denominators.
- Current source authorization and immutable phase/source hashes before and after every call, including failure paths. No transaction spans inference. Historical `Memory::read` alone does not establish freshness. No writes during paired decisions.
- Raw output, structural acceptance, correct choice, premise coverage, prose support and cost are distinct measurements. The native evaluator does not grade this experiment. Preserve old cohorts.
- The post-phase measures revised-concept utility versus equal raw evidence. Without a third unchanged-concept arm, it cannot isolate the causal effect of revision itself; different initial/post cases cannot establish a before/after improvement.

## Review focus

1. Gold or future-case leakage into requests: inspect actual serialized requests, not just fixture types.
2. Source/scope/database mutation during discovery or inference: deny dispatch/finalization, including transport-failure paths.
3. Partial completion: create all planned rows before inference; preserve successful retention even if later export fails; never replay an indeterminate slot.
4. Concept authority: provenance and exact quotes are required, but neither admission nor a generated principle certifies truth or authorizes action.
5. Confounded treatment: same original evidence and current facts per pair; unchanged concept after rumination labeled honestly; missing information scored from observable premises.

## Shared fixture interface

Public JSON schema: `{version:1, training:[{id,text},{id,text}], counterexample:{id,text}, cases:[{id,phase,question,options:[string],facts:[{id,text}]}]}`. `phase` is `initial` or `revised`, exactly three cases each; IDs unique and bounded. Every options list includes `insufficient_evidence`. Plain source/current-fact IDs are citable aliases with host-recorded original revision bindings. Neither concepts nor their names are independent premise IDs.

Decision response: `{request_key, choice, because:[original_id], reason, question}`. `question` is nullable; insufficient-evidence answers require a nonempty question. Both arms share a schema except their bound request identity. Limit reason to 512 characters, question to 160, premise IDs to 12; reject duplicate/foreign IDs and unknown fields. Host acceptance proves contract adherence only.

The workflow owner publishes a report-format note before scorer integration. Each planned stage records ID/kind/phase/case/arm, terminal or withheld status, exact request/response artifacts and hashes, nullable usage, timings, admission/retention outcome, and error. Frozen fixture and binary hashes, model settings, planned/actual counts and pair evidence hashes are retained. The scorer validates references and reports all planned rows; no missing row is silently dropped.

### Task 1: Independent cases and deterministic scorer

Files: create `mecha-core/src/memory/fixtures/concept_benefit_pilot.json`, `prototypes/memory/fixtures/concept_benefit_pilot_gold.json`, `prototypes/memory/concept_benefit_trial.py`, and `prototypes/memory/test_concept_benefit_trial.py`.

Interfaces: public fixture above; offline scorer consumes the workflow report and independently frozen gold. Coordinate the report adapter with Task 2; gold is never loaded by Rust or sent to the model.

- [x] Author two synthetic episodes about bypassing a route-specific obstruction, plus a counterexample involving a shared prerequisite. Author six new cases: applicable, near-match, uncertain in each phase. Explicit current observations must justify the expected answer.
- [x] Define independent finite-world parameters and compute feasible/preferred alternatives; enumerate permitted completions for unknown observations. Freeze tie handling and sufficient original-premise sets. Document any ceiling limitation.
- [x] Write and observe failing tests for wrong choice, missing premises, false certainty, absent rows, tampered artifacts, rejected responses, and unchanged-versus-revised follow-up labeling.
- [x] Implement minimal stdlib scorer and fixture validation; preserve planned denominators and separate raw/accepted scores, costs, and unreviewed prose.
- [x] Run scorer suite; report files, commands and failure evidence for independent review. Do not commit: parent performs strict Spark privacy scan first.

### Task 2: Core lifecycle and thin executable

Files: create `mecha-core/src/memory_concept_workflow.rs` plus focused child modules/tests as needed, `mecha-core/examples/memory_concept_trial.rs`; feature-gated registration in `mecha-core/src/lib.rs` only. Existing concept APIs, schema and shared transports remain unchanged unless an observed integration defect requires a reviewed correction.

Interfaces: consume public fixture above; expose one workflow function accepting fresh root, fixture, `StageProviders`, `GenerationBudget`, and cancellation, returning a serializable report. Use existing `memory_support_workflow`, `memory_factual_support::pass`, and recorded provider facilities. Publish exact report layout to Task 1 early.

- [x] Write failing scripted lifecycle tests: induction retained/reopened, initial pair equivalence, exact counterexample revision with three-source closure, new pairs, all fourteen slots and no gold/future-case leakage.
- [x] Implement fresh-root capture, one-call induction, paired decisions, one-call rumination and post-phase orchestration. Retain exact raw replies and immutable phase manifests. No evaluator/model-as-judge or action dispatch.
- [x] Test malformed/refused/model-mismatch/timeout paths, induction and revision failure withholding, unchanged/abstained no-write outcomes, independent sibling allowance, cancellation, and source/scope/revision mutation before and after inference. Record successful writes before exporting derived reports.
- [x] Add the thin explicit-synthetic example with resident-only provider and Ctrl-C cancellation, matching existing local trial conventions.
- [x] Run focused tests/fmt/Clippy; report red-to-green evidence and limits. Do not commit or run inference; parent owns final qualification, freeze, privacy scan and publication.

### Task 3: Qualification, bounded experiment and publication

Files: update existing design, architecture, handoff/history, prototype README, changelog and living Site; ignored `results/memory-concept-benefit-20261108/` holds raw experiment artifacts.

- [x] Verify clean-base workspace build and tests before implementation. Independently review both tasks and the integrated workflow against this spec.
- [x] Run fmt, strict all-target/all-feature Clippy, workspace build, default and memory-enabled suites with required backends, standalone and actual-binary Python checks. Compile actual scripted-lifecycle schemas with the pinned engine converter before real inference; compile and audit real-cohort schemas afterward because rumination depends on the induced concept. Preserve real schema failures without retries.
- [x] Freeze public fixture/gold/scorer/binary/source/settings and expected outcomes before model responses. Run at most fourteen first responses on isolated Spark, no simultaneous builds or tuning. Audit source/DB integrity and original artifact hashes independently; score choice/premises with the offline oracle and prose separately.
- [ ] Report negative/ceiling/partial results honestly. Update docs, PR and private Site; strict Spark privacy scan exact diff/message before commit/push. Follow CI/automatic review and fix verified major/medium findings. Do not merge or activate production.
