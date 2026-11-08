# Native evidence and provisional concept rumination implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Implement only the assigned task and commit it in an isolated worktree.

**Goal:** Preserve complete native evidence for read-only appraisal, then begin evidence-preserving revision of provisional concepts.

**Architecture:** Two independent, default-off core APIs reuse existing authorization and persistence. Native appraisal selects authenticated fact IDs without persisting judgments. Concept rumination revises one provisional concept against its original evidence plus explicit counterexamples, with no automatic rule installation.

**Tech Stack:** Rust, SQLite, serde, existing memory-prototype feature and provider traits.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` sections 31.9, 31.14 and 31.15; `docs/ARCHITECTURE.md` Experimental memory prototype.

## Global constraints

- Preserve v4 canonical contracts, historical results, source taint and complete consulted dependencies.
- No database migration, production configuration change, owner graph replacement, or nightly-job migration.
- Native source reader guards do not authorize concept promotion; initial concept rumination uses unbound synthetic episodes.
- No transaction across inference. Recheck every dependency before dispatch and after inference.
- Model judgments and concepts remain provisional; structural validation is not semantic qualification.
- Whole values fit or are explicitly omitted; no silent truncation. Unknown times remain unknown.
- Parent owns shared documentation, integration, final tests, PR and Site updates.

## Review focus

- JSON-looking untrusted text must never impersonate a host-native envelope.
- Revoked uncited dependencies must invalidate both appraisal and concept revision.
- A stale prior concept must not be revived by a no-change or abstention outcome.
- Complete empty artifact bytes must remain distinct from absent or refused observation.
- Concept revision must not erase inconvenient counterexamples or quietly change source scopes.

### Task 1: Typed native evidence and provisional read-only appraisal

**Files:** Create `mecha-core/src/memory/native_evidence.rs` and tests; include it from `memory/interpretation.rs`; minimally reuse the source recheck in `memory/support.rs`. Add a provider adapter using existing budget/cancellation machinery if needed. Do not change existing v4 wire meanings.

**Interfaces:** Opaque `NativeEvidenceRequest`, serializable `NativeEvidenceView`; `Memory::prepare_native_evidence(&PreparedInterpretation, &Access)` and `Memory::finish_native_appraisal(&NativeEvidenceRequest, response, &Access)`. Exact response and view types belong to this module.

- [x] Write failing tests for complete native values, provenance IDs, ordinary JSON text, status distinctions and response replay.
- [x] Decode only authenticated roles/fields into existing host envelope types. Preserve source revision, field, selector, full metadata and criterion; bind complete view to request identity.
- [x] Reuse preparation recheck before exposure and finalization; bounded strict response selects only issued IDs. Known states require premises, unknown requires a question.
- [x] Test schema compilation, foreign/duplicate IDs, malformed envelopes, stale/revoked uncited dependencies, and read-only behavior. Provide matched raw-versus-typed request interfaces for a model comparison.
- [x] Run focused tests, self-review and commit; report exact commands and limitations.

### Task 2: Evidence-preserving provisional concept rumination

**Files:** Create `mecha-core/src/memory/concept_rumination.rs` and tests, or a child module of `concept.rs` to reuse private admission helpers. Only minimal declarations in `memory/mod.rs` or `concept.rs`. Do not touch native appraisal files.

**Interfaces:** Opaque preparation receipt binding expected concept revision, admitted source union, previous concept and mandatory counterexamples. Prepare/finish and one-call async convenience APIs; bounded revised-provisional, unchanged, or abstained outcomes with no authority promotion.

- [x] Write failing lifecycle tests using existing synthetic captures and concept retention.
- [x] Union every prior consulted revision with explicit new counterexample sources; deduplicate and refuse more than eight. Validate exact counterexample excerpts before inference.
- [x] Include prior provisional concept as untrusted hypothesis. Require preservation of prior and supplied counterexamples; never let model remove these obligations.
- [x] Recheck prior revision and all sources on every outcome; retain revised draft through existing optimistic concept retention. Invalid output writes nothing; no retries or installed instructions.
- [x] Test source/scope changes, forgetting, stale revisions, dropped counterexamples, recall with full evidence, independent episodes and exactly one provider call. Add a runnable synthetic lifecycle demonstration if useful.
- [x] Run focused tests, self-review and commit; distinguish lifecycle mechanics from unmeasured model generalization.

### Task 3: Qualification, integration and publication

**Files:** Existing synthetic trial/audit scripts and regression tests as needed; existing design, architecture, handoff, history and changelog documents; private Site research source.

- [x] Integrate independently reviewed commits and fix the missing `interpretation_roles` audit diagnostic with a regression.
- [x] Run a small fresh, fixed raw-versus-typed model comparison on Spark if resident service allows; preserve requests/responses and account for failures. Do not call development fixtures held-out evidence.
- [ ] Run required formatting, strict Clippy, workspace/memory tests, standalone compatibility, Python checks and final privacy verification. All software checks pass; final documentation/message privacy check remains.
- [ ] Review final increment, update source-verified handoff and PR #846, inspect automatic review/CI, then publish accurate private Site progress. No merge or production restart.

**Experiment limit:** the fixed pilot stopped after seven replies because an owner model switch blocked new holds. Missing slots, complete artifacts and source/binary/fixture checks are preserved; full semantic qualification is not complete. See HISTORY.
