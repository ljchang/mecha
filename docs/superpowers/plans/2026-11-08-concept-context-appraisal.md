# Larger Memory Context and Concept Appraisal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Recall larger native evidence bundles without losing lineage, and appraise with provisional concepts without promoting them to facts; then qualify encrypted storage and recovery.

**Architecture:** Keep canonical SQLite records intact. Compact only authenticated reader presentations. Extend native checks-first appraisal through an explicit hypothesis-aware contract, reusing factual validation and the existing executor. Encryption and recovery follow these reviewed changes; vector retrieval remains a subsequent measured improvement.

**Tech Stack:** Rust, rusqlite/SQLite, native JSONL, existing GoalMemory and native evidence APIs; qualify SQLCipher after auditing mecha-graph reuse.

**Spec:** docs/MECHA-MEMORY-DESIGN.md, especially encryption/dependencies, retrieval/caching and native-session/concept sections; user-approved sequence on 2026-11-08.

## Global Constraints

- Explicit synthetic prototype only; no live private imports, production activation, merging or automatic nightly migration.
- Preserve complete provenance, restrictions, taint, source liveness, snapshot semantics and unknown/omitted distinctions.
- The existing 20,000-byte packet cap stays unchanged. Measure serialized UTF-8 bytes, not characters.
- No excerpts or hidden removal of source text in the first compact presentation. Mandatory material that cannot fit is withheld.
- Concepts remain provisional hypotheses. Their names/principles cannot serve as factual observation IDs or supported judgments.
- Legacy protocols and canonical stored records retain their behavior and identity.
- No implicit model selection/loading; real inference only on the user-selected resident Qwen with existing holds and no-autoload transport.
- Separate worker worktrees, serial implementation, independent task review. Parent owns shared documentation, PR and Site.
- Baseline workspace build/tests must pass before product edits. Strict privacy scan before commits/pushes; no private data or keys in artifacts.

## Review Focus

1. Recapture of a compact packet must preserve every dependency and never recursively store the packet.
2. Canonical JSON-looking prose must not acquire authenticated native metadata roles.
3. Uncited source or concept-only scope withdrawal must invalidate prepared and final outputs.
4. Hypothesis identifiers must be rejected as factual premises, including cross-protocol or request-key substitution.
5. Encryption failures must never open/create plaintext; backups must restore a consistent usable store without leaked keys.

### Task 1: Compact authenticated native context

**Files:** mecha-core/src/memory/context.rs, interpretation_reader.rs or a new native reader module, memory_experience_bridge.rs; focused tests and memory_integrated_native tests.

**Interfaces:** Keep ContextPacket.records as canonical Vec<Hit>. Add an internal prepared presentation constructed within the authorized read transaction. Use exactly that representation for byte admission and final serialization. Bridge decoding recognizes the versioned representation separately from canonical records. Preserve original text in full; compact authenticated projection metadata into typed JSON rather than an escaped repeated string.

- [x] Reconstruct the fuller-citation authored revised-concept case whose canonical packet exceeds 20,000 bytes; observe budget omission before implementation.
- [x] Implement a versioned typed reader for authenticated native capture metadata, retaining complete criterion, action/check status, original roles and consulted lineage. Keep canonical export unchanged.
- [x] Align explicit native capture/recapture dependency bounds with the existing bounded 64-record concept closure; reject 65 rather than truncate. Audit each existing 32-reference limit before changing it; do not change unrelated bounds.
- [x] Test exact serialized bytes, escaped/UTF-8 source text, duplicate bundle union, successful recapture with identical complete dependency set, and no nested packet copies.
- [x] Test 32/33/64/65 boundaries, forged roles/references/versions/taint, unavailable uncited dependency, and oversized mandatory content withholding. No large-source excerpt claim.
- [x] Run focused RED/GREEN tests, self-review and report. Independent review follows before integration.

### Task 2: Explicit concept-aware native appraisal

**Files:** mecha-core/src/memory/native_evidence.rs, native_appraisal_checks.rs, interpretation.rs (private preparation only), new native_concept_appraisal module/tests; opt-in core workflow integration as needed.

**Interfaces:** An opaque NativeConceptEvidenceRequest prepared from the same capture/context/access inputs as prepare_native_evidence; explicit prepare_native_concept_evidence, recheck_native_concept_evidence and finish_native_concept_appraisal APIs. Use protocol situated-native-concept-appraisal-v1. Add an explicit appraise_native_session core workflow over database, LocalAuthority, host-owned request, Provider and cancellation; reuse memory_factual_support::pass without another executor or implicit discovery. Reuse native fact decoding and checked-appraisal validation. Expose facts and provisional hypotheses separately. Each hypothesis retains canonical concept revision, applicability, supporting and opposing citations. Response includes per-hypothesis applicable/inapplicable/unknown assessment with factual IDs as reasons. Use a distinct protocol/request identity.

- [x] Observe old native preparation refusing a concept-bearing closure; write separate positive and adversarial tests for the new explicit path.
- [x] Partition canonical concepts from facts only in private opt-in preparation; do not make Concept generally acceptable to source_fields_impl or v4 support retention.
- [x] Reconstruct canonical concept provenance and authorize the complete union before dispatch and after inference, including omitted/uncited sources and concept-only scopes.
- [x] Reject concept IDs in because/artifact_facts; reuse checks-first criterion/artifact consistency. A concept-only explanation cannot establish a known factual outcome.
- [x] Add an authored reopened native-session integration showing relevant concept use, changed-goal inapplicability, counterexample visibility and withdrawal. Keep hypothesis influence in recapture lineage and exclude it from independent induction support.
- [x] Test tampering, cross-protocol requests, duplicate keys, stale/forgotten sources, scope changes and database replacement. Old APIs remain unchanged.
- [x] Run focused RED/GREEN checks and independent task review; no claim of semantic reliability from authored responses.

### Task 3: Integrated qualification and publication

**Files:** existing prototype/design/architecture/handoff/history/changelog documents and private research Site.

- [x] Integrate reviewed patches and run authored end-to-end evidence checks under the unchanged packet limit.
- [x] Run workspace build, formatting, strict Clippy, default and memory-feature suites and applicable actual CLI checks on isolated Spark; preserve exact-source manifests.
- [x] Independent combined review; fix major/medium defects and qualify changed behavior.
- [x] Run handoff and privacy procedures, update draft PR and Site with verified outcomes and remaining cutover checklist; finish exact-head CI/review loop.

### Task 4: Encryption and recovery qualification

**Files:** selected after read-only mecha-graph audit; storage connection boundary, shared Cargo SQLite linkage, key handling and backup/restore APIs/tests.

**Interfaces:** Preserve current explicit synthetic plaintext entry points. Introduce explicit protected-store entry points with externally supplied key material, no fallback, and verified encrypted backup/restore. Determine exact signatures in a follow-on plan from the reuse audit before changing dependencies.

- [x] Inventory mecha-graph encryption, key storage, backup/restore and on-demand embedding client; document reusable pieces and incompatible assumptions.
- [x] Write the bounded follow-on implementation plan after predecessor code qualification and review; specify the candidate SQLCipher linkage and its pending compatibility qualification.
- [ ] Test missing/wrong keys, no plaintext fallback, WAL/backup protection, restore/reopen integrity, interrupted operations, permissions and legacy persona DB compatibility.
- [ ] Keep JSONL/source protection and key recovery requirements explicit; database encryption alone does not qualify private-data migration.
- [ ] Review, qualify, update PR/Site; defer vector indexing to its own measured hybrid-retrieval increment using the existing embedding service where compatible.

Protected storage continues in `2026-11-09-protected-memory-storage.md`. Its tests
were prepared while the predecessor qualified; product changes remain gated on
reviewed source qualification. Publication and exact-head CI are tracked in PR #846.
