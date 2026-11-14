# Compact appraisal recall implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Execute task-by-task with regression evidence and independent review.

**Goal:** Make complete atomic-appraisal evidence recallable without duplicating canonical audit data, and distinguish successful storage from unsuccessful recall.

**Architecture:** Keep full canonical v8 records in SQLite and internal ContextPacket hits. Serialize atomic v4 interpretations through an explicit compact reader DTO that preserves the full SupportReport and source locators. Native recapture recognizes that representation explicitly. The trial records storage, canonical export and ordinary recall separately.

**Tech Stack:** Existing Rust/serde/rusqlite core, Python independent audit; no new dependency or database migration.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` sections 31.12 and 31.13; current failure evidence in HISTORY. Approved continuation of bounded retrieval/evidence repair; semantic native-view redesign remains the next separately qualified increment.

## Global Constraints

- Synthetic default-off prototype only; no production service, owner store, nightly activity, merge or schema migration.
- Preserve the 20,000-byte contextual packet cap, 32-record limit, complete source closure, fresh final authorization, source filters and private/untrusted taint.
- SQLite payloads, receipt hashes, v4 inference prompts/bindings/reports and all prior experimental artifacts remain unchanged.
- Compact only canonical atomic-v4 records; preserve legacy record serialization. A presentation must never deserialize as a canonical Record.
- User authorized parallel isolated implementation and PR/Site updates. Parent owns shared docs, integration, privacy scans, commit/push; workers return patches and do not commit/push or spawn agents.
- No new model calls in this mechanical repair. Existing eight cases are development regressions, never new semantic evidence.

## Review Focus

- Source withdrawn between selection and final return must withhold the compact bundle; serialization is not authorization.
- Large Unicode/escaped quotations must obey actual serialized bytes, without truncation or missing uncited sources.
- Compact packets recaptured through native GoalMemory retain every dependency and current/historical role.
- Storage succeeds but recall fails: receipt/database/canonical audit survive and stage diagnostics name the actual failure.
- Legacy packets and frozen scores remain readable unchanged; new representation gets strict validation and explicit protocol identity.

### Task 1: Explicit compact reader and native recapture

**Files:** memory/context.rs, new memory/interpretation_reader.rs and memory module declaration; memory_experience_bridge.rs; relevant memory interpretation/support and bridge tests.

**Interfaces:** ContextPacket.records stays Vec<Hit>. A field serializer uses a public or crate-visible strictly typed `InterpretationReaderV1` DTO and distinct record kind `interpretation_reader_v1` for atomic-v4 records only. DTO fields: context_key, criterion, host_context_id, producer_version, claim_sources, support. The full unchanged SupportReport supplies observations, original citations, dispositions, judgment premise IDs and stage provenance. Add pub(crate) Memory::stored_interpretation(&self, receipt: &InterpretationReceipt, access: &Access, context: &InterpretationContext) -> Result<Interpretation> for host-only audit: authenticate active receipt identity/context, canonical payload, full source closure and final reopened authorization, without the reader packet cap. Other record kinds serialize as before. Expose the DTO/conversion where the bridge needs it; never modify global Record serialization.

- [x] Write and run RED regression: a valid large nine-record atomic closure exceeds 20 KB under legacy serialization but ordinary contextual recall must return all records under 20 KB. Include actual GoalMemory serialization.
- [x] Implement compact serialization; retain full canonical records internally and unchanged SQLite bytes/hashes.
- [x] Extend strict native packet decoding with a separate compact record branch, preserving duplicate-key/field/role/context/taint/reference checks and full consulted dependencies. Unknown versions fail closed.
- [x] Test exact compact report/locators, canonical Record rejection of reader kind, old serialization unchanged, smaller/too-large packet limits, current/historical roles, uncited-source revocation/final authorization, and native recapture followed by source withdrawal. Reuse existing fixture/guard tests where suitable; no vacuous mocks.
- [x] Run focused core/bridge tests, fmt and report precise RED/GREEN evidence. Return owned-files patch including new files.

### Task 2: Separate persistence diagnostics and audit compatibility

**Files:** memory_atomic_workflow.rs, memory_support_workflow.rs only where a narrow helper is required, prototypes/memory/atomic_trial.py, tests/test_atomic_trial.py. Avoid changing task1 files or inference policy.

**Interfaces:** Existing `run_case`/PROTOCOL remain. Additive artifact/status fields distinguish storage receipt, canonical stored-record export and contextual recall; a stored_export stage may be added while the auditor continues to accept archived stage sets. Preserve existing successful outputs and archived report parsing. Compact recall record shape is `kind=interpretation_reader_v1`, data exactly task 1 DTO fields; auditor reconstructs that expected representation independently from canonical retained export.

- [x] RED tests: after successful retention but contextual-reader exclusion, report preserves receipt, immutable database snapshot and canonical stored-record export, marks retain completed and recall failed, exports available budget_omitted/selected_current/packet-byte diagnostics. A failed export must not pretend successful verification.
- [x] Move canonical audit extraction out of the budget-limited contextual reread. Authenticate the stored interpretation through the existing canonical guarded read surface; retain complete final authorization. Do not use raw SQL as source authority. Ordinary reopened GoalMemory still establishes actual recall separately.
- [x] Independently validate both legacy full and explicit compact recall presentations against canonical retained data, rejecting altered report/locators/context/producer or unknown shape. Do not change frozen semantic thresholds or manufacture semantic annotations.
- [x] Verify original frozen audit still reproduces its prior results. Test post-write failure accounting and control behavior with authored providers, including oversized legitimate bundles.
- [x] Run focused Rust/Python tests and return owned-files patch with RED/GREEN report.

### Task 3: Integrated development replay, review and publication

**Files:** existing design/architecture/handoff/history/prototypeREADME/changelog/index; isolated result artifacts and Site source.

- [x] Integrate reviewed task patches; independent final review, fixes, fmt/strictClippy/default and memory-enabled workspace suites plus standalone/Python/native-contract checks.
- [x] Reopen all eight archived v4 databases through the real compact contextual reader with original LocalAuthority, criterion/context and source dependencies; preserve originals and verify hashes before/after. Record selected role, exact bytes, full-reference equality and canonical record identity. If source paths are unavailable report exclusion, never install an allow-all guard. No new inference or changed labels.
- [x] Document structural repair separately from unchanged semantic failures. Add concrete next design for core-derived whole native values and versioned full locators, preserving current/historical/request/observation distinctions; no claim this is implemented.
- [ ] Required Spark owner-corpus privacy scan on exact staged diff and commit message, commit/push existing draftPR846, inspect automatic review and CI. Update/publish private research Site; preserve audience and old results. No merge.
