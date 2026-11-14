# Protected Mecha Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Make the explicit synthetic memory workflow usable with encrypted SQLite, then provide verified encrypted snapshots and fresh-path restore that preserves current forgetting.

**Architecture:** Keep domain/scopes separate from an opaque host-owned storage capability. Key every independent connection before its first read. Export complete snapshots through a private connection, verify them independently, and publish without overwriting. Restore uses the owner-selected current store as deletion and access-policy authority; it cannot recover safely from an old backup alone.

**Tech Stack:** rusqlite 0.41.1 / one locked libsqlite3-sys 0.39.1 linkage, bundled sqlcipher-src 419.0.6 (SQLCipher 4.19 / SQLite 3.53.4) with vendored OpenSSL, getrandom 0.3 and zeroize 1. No graph crate, vector dependency or new workspace crate.

**Spec:** docs/MECHA-MEMORY-DESIGN.md §31.17 and protected-storage addendum; audited implementation seams in results/memory-concept-context-20261108/encryption-implementation-brief.md, including its final native-session refinement. The latter is local working evidence; this plan preserves the binding requirements for other checkouts.

## Global Constraints

- Start product changes only after compact-context/concept-appraisal source qualification and review pass. Existing default-off synthetic plaintext APIs remain compatible.
- No live private import, production activation, merge, live replacement, model discovery/switch/load or network call inside storage transactions.
- Host-only storage capability: no key material in DTOs, model requests, logs, reports, error chains, argv or automatic environment/sibling-file discovery. Key paths are explicit owner CLI input.
- Preserve schema versions, instance/domain identity, scopes, taint, source freshness, snapshot comparisons and final independent reopens. No plaintext fallback and no schema mixing/upgrades.
- All new database outputs are exclusive new paths. Read-only public handles stay query-only. Encryption does not protect JSONL, authority files, task artifacts or provider audits.
- Restore requires accessible current deletion authority. No claim of full disaster recovery, key recovery, secure erasure of historical copies or later cross-copy deletion propagation.
- Serial product implementation in isolated worker worktrees, independent review per task; parent owns docs, PR, Site and final strict privacy/commit. Tests use invented data only.

## Review Focus

1. A correct initial key must reach every later guard/tool/appraisal reopen, without caching authorization.
2. Wrong keys or unsupported CLI options fail before provider construction or task/output creation, and chained errors reveal no key.
3. A destination opened for export must be writable without changing the source or weakening public readers.
4. Backups preserve all tables, indexes and sequence metadata; a deleted high sequence and FTS shadow tables cannot silently disappear.
5. Restoring an older snapshot must not resurrect forgotten descendants, including an object created and forgotten after that snapshot.
6. An older snapshot must not roll back tightened access scopes. Current-head scope checks are required independently of deletion epochs.

## File responsibilities

- `memory/storage.rs`: opaque key capability, strict file policy, key generation/loading, key-first connection helpers.
- `memory/store.rs`, `sql.rs`, `interpretation.rs`, durable guard modules and `memory_outcome_reappraisal.rs`: carry capability through existing identity and authorization boundaries.
- `tool/goal_memory.rs`, `memory_situated_workflow.rs`: host-only capability for native execution, recall, capture and separate appraisal.
- `mecha-cli/src/commands/memory.rs`: explicit selection/routing, supported command checks and safe key exclusions.
- `memory/recovery.rs`: private export connection, canonical inventories, encrypted backup and gated restore; focused submodules if needed.
- `memory/forget.rs`: one shared transactional purge used by ordinary forgetting and restoration.
- Focused storage/recovery tests plus native integration and existing plaintext subsystem suites establish behavior.

### Task 1: Encrypted connections through the native workflow

**Files:** Cargo.toml/Cargo.lock, mecha-core/Cargo.toml, prototypes/memory/Cargo.toml and its Cargo.lock; new memory/storage.rs and storage tests; memory/{mod,store,sql,interpretation,durable,durable_decision,durable_action}.rs; memory_outcome_reappraisal.rs; tool/goal_memory.rs; memory_situated_workflow.rs; mecha-cli/src/commands/memory.rs; native integration tests. HUD runner/authorizer/tests for the wrapper API change and new function denial; packaged `mecha-core/THIRD_PARTY_NOTICES.txt` for SQLCipher/OpenSSL licenses.

**Interfaces:** `StorageAccess::{plaintext_fixture,load_key_file,generate_key_file,is_encrypted}`, `KeyFilePolicy::new(&[PathBuf])`; `Memory::{open_with_storage,open_read_only_with_storage,create_supported_situated_with_storage}` and internal `create_schema_with_storage`, `reopen_read_only`, `storage_access`. Existing wrappers choose explicit plaintext. Add `GoalMemory::open_with_storage_and_source_guard`. Add `run_native_session_with_storage` and `appraise_native_session_with_storage`, keeping existing signatures as plaintext wrappers; storage is a separate host argument, never a request field. Durable v3–v5 constructors gain access-aware siblings or private test construction using their actual schemas.

- [x] Freeze old-engine synthetic plaintext fixtures and exact generator/version before switching the dependency. Reuse the prepared fixture evidence where applicable; no real persona data.
- [x] Write meaningful failing tests for cipher capability, encrypted create/reopen/forget, missing/wrong/malformed key refusal without mutation, positive keyed guards and native session/appraisal reopens. Confirm the old implementation cannot satisfy encryption, not merely a missing-method compiler error.
- [x] Upgrade to rusqlite 0.41.1 with locked libsqlite3-sys 0.39.1 and its exact sqlcipher-src 419.0.6, replacing `bundled` with `bundled-sqlcipher-vendored-openssl` while retaining hooks/limits and one sys linkage; add direct getrandom/zeroize dependencies. The standalone harness compiles the same memory module by path: align its cipher feature and direct dependencies too, preserving its independent workspace and existing dead-code qualification allowance. Confirm downloaded registry source hashes, runtime SQLCipher 4.19 and SQLite 3.53.4, unchanged plaintext behavior and published MSRV before proceeding. Adapt only affected rusqlite API call sites (including column iterator changes), with existing regression coverage. Do not adopt the cached SQLCipher 4.14/SQLite 3.51.3 bundle: it lacks later export/URI fixes and predates the current ordinary SQLite baseline. The official 0.41.1 build script uses sqlcipher-src despite an older SQLCipher version mentioned in release-note prose; actual registry source and runtime decide the gate.
- [x] Implement strict 32-byte key loading from exactly 64 ASCII hex bytes plus optional newline. Read a bounded no-follow descriptor; reject symlink/nonregular/permissive files, nonprivate parent, unsafe containment and malformed/oversized input. Validate canonical roots and existing components; document the same-user ancestor-swap limitation. On macOS, inspect parent and loaded/new key descriptors for extended ACLs and ownership-disabled volumes; refuse either or inspection errors before reading/writing key material. Do not broaden permissions in tests; use restrictive ACL fixtures and clearly labeled injected metadata checks where appropriate. Capability uses Arc-backed zeroizing secret, no serialization and redacted Debug/errors.
- [x] Generate with OS randomness and exclusive 0600 creation, file/parent sync, no overwrite or directory chmod. Test write/sync failure and collisions; distinguish a complete orphan key from a valid database, without logging its contents.
- [x] Preserve model-authored HUD SQL restrictions under the shared cipher engine: explicitly deny `sqlcipher_export` case-insensitively in the existing authorizer, test policy denial and actual prepare-time rejection, and verify cipher_log/default PRAGMAs refuse without filesystem side effects. A read-only engine error later is not equivalent to refusing the function at preparation.
- [x] Key before schema/identity/journal queries; require a supported stable SQLCipher 4.x version at least 4.19 and successful schema read. Reject nonempty older/unknown versions and 5.x beta; test the version gate so a system-link override cannot silently admit the cached older codec. Enforce/read back secure_delete, DELETE journal, FULL synchronous and memory temp storage. No process-global cipher defaults or unkeyed retry.
- [x] Carry capability through all three interpretation reopens, DispatchGuard, ModelGuard, ActionGuard, OutcomeGuard, GoalMemory, native preflight, execution recall, post-run capture and separate concept appraisal. Preserve independent transactions and source guards. Test positive v3–v5 guard paths and v8 capture, plus replacement/epoch/source/scope failures.
- [x] Extend existing CLI with explicit --key-file and repeatable --key-exclude-root, key-generate, and supported InitNative/NativeSession/read/search/forget/audit routing. Exclude cwd, database/export/authority/task roots. Exhaustively reject encrypted selection for unconverted adapters before side effects. Never add keys to model-visible arguments.
- [x] Exercise an authored encrypted native session, fresh-process/handle recall and later session, failed artifact check capture, interpretation-aware recall, separate concept-aware appraisal, no replay and source withdrawal. Wrong key must leave factory count zero and no started root. Jailed file tools cannot read the external key; reports/audits contain no key encoding.
- [x] Run focused RED/GREEN, legacy plaintext subsystem/fixture checks, core memory/native integration, CLI tests, fmt and strict Clippy. Qualify Linux/macOS, supported release/MSRV/package checks where available and report unavailable targets explicitly. Review exact diff; parent commits only after strict privacy.

### Task 2: Complete encrypted snapshots

**Files:** new memory/recovery.rs and focused recovery tests/submodules; memory/mod.rs; CLI memory backup command.

**Interfaces:** `Memory::backup_encrypted(&self, destination: &Path, destination_storage: StorageAccess) -> Result<BackupReport>`. Public acceptance is encrypted v8 only. Private snapshot/export helpers are reused by Task 3; report contains nonsecret counts/versions/scope only.

- [x] Write failing export/integrity/source-immutability tests using different source/destination keys and populated v8 tables, FTS, receipts, restrictions and deleted-high AUTOINCREMENT sequence.
- [x] Implement a private writable staging-main connection with keyed source attached using explicit read-only URI; qualify `sqlcipher_export('main','source')` with a pinned source snapshot. Do not reuse/toggle public query-only handles. Bind paths/raw-key representation, sanitize SQL errors, configure memory temp/durability and close attachments before publication.
- [x] Inventory schema/columns and every canonical/shadow table using deterministic typed length-prefixed row digests (NULL/text/blob/integer/real distinct), excluding allocation rootpages. Preserve application_id, user_version, identity/epochs and sqlite_sequence. Verify a deliberately omitted ordinary or shadow table fails verification.
- [x] Reserve staging exclusively at mode 0600, using a filename not accepted as a normal prototype store. Independently reopen and verify cipher HMAC, integrity, foreign keys, application schema and complete inventory against the same source snapshot. No plaintext row manifest.
- [x] Sync then publish with atomic no-clobber semantics and directory sync. Test competing publishers, copy/verify/file-sync/publish failures and post-publication durability uncertainty. Clean only owned staging paths; preserve existing outputs and source bytes. Bounded writer contention must not create a torn snapshot.
- [x] Add explicit CLI backup output/key arguments and help stating database-only scope. Test subprocess invocation, unsupported modes, source/key isolation and no secrets in full errors. Review and run focused plus affected regression tests before integration.

Tasks 1 and 2 are implemented, independently reviewed and locally qualified. Task 1 also passed its scoped Linux/MSRV gate. Final combined qualification is recorded under Task 3. Strict privacy review and publication remain separate release gates.

### Task 3: Restore without resurrecting forgotten memory

**Files:** memory/recovery.rs and tests; memory/forget.rs shared purge; CLI restore routing/tests.

**Interfaces:** opaque `RestoreAuthority::current(path, storage)`; `restore_encrypted(backup, backup_storage, destination, destination_storage, &current) -> Result<RestoreReport>`. Internal `SuppressionState` includes instance/domain/sequence/epoch, digest set and restricted object id/kind map. `purge_seeded` runs the existing deletion fixed point under its caller's transaction without committing or choosing epochs.

- [x] Write backup→forget→restore regressions covering canonical/quoted descendants, concepts, sibling captures/receipts, FTS, reingest suppression, and a post-backup-created-and-forgotten object absent from backup.
- [x] Refactor the existing forget purge once, retaining all host/durable lifecycle expansions and invalidations, ordinary forget epoch/commit/VACUUM behavior. Both digest matches and restricted IDs seed purge.
- [x] Reopen owner-selected current encrypted v8 authority and hold bounded BEGIN IMMEDIATE during local maintenance. Require matching instance/domain/schema, backup sequence/epoch no newer, backup suppression/restrictions subsets of current. Check authority path/identity before publication. Missing/corrupt/wrong-key/foreign/known-stale authority refuses.
- [x] Export old snapshot to a fresh encrypted stage; add missing opaque object(id,kind) stubs for restriction FKs, rejecting conflicting kinds. Apply current suppression/restrictions and actual purge; retain additional suppressions from sibling invalidation, preserve instance and original IDs, advance epoch to current and rebuild/scrub/VACUUM.
- [x] Under the same current-authority transaction, compare current-head required scopes against restored-head scopes for every surviving object. Refuse if restoration would omit any currently required scope or the current head is unavailable; already purged restricted objects are exempt. Check every surviving object, including dependency-only inputs. Do not copy newer factual payloads or invent a policy overlay to bypass refusal. Regress backup → tighter scope with unchanged security epoch → restore refusal; also check unchanged scopes, stricter backup scopes, missing current heads and writer contention.
- [x] Independently verify transformed state, no readable restricted revision and blocked reingest before publication. Use Task 2 no-clobber/durability protocol. No process-local permit is recreated and no live path is replaced.
- [x] Test busy authority, changed authority path, failures during purge/verification/publication, unavailable authority, all key combinations and retained older copies. CLI/help/report say fresh offline historical copy, current-authority required, and include snapshot/authority sequences. Later factual/concept revisions and withdrawals are not merged: an old head is locally current within the restored snapshot. External sources and future deletion propagation are not restored/provided; no live installation or current-knowledge claim.
- [x] Review exact diff, run focused/regression checks and complete default/memory workspace, fmt/strict Clippy, actual CLI and release/package qualification.

**Publication procedure remains required:** the parent updates handoff/history/design/changelog, runs the strict Spark source/message privacy scan on the exact final staged content, updates the draft PR/private Site and closes the latest-head CI/review loop. Current publication status is recorded on PR #846 and the private Site; this implementation checklist is not a substitute for those gates.

**Combined qualification:** Spark run `combined-20261109T071728Z` passed default 6,354/0/25 and memory-enabled 7,008/0/30 (passed/failed/ignored), standalone 372/0/0, 29 actual CLI checks, strict Clippy, Rust 1.96.1 checking and package verification. The unchanged combined source also passed the optimized macOS CLI build and 29-check smoke. HISTORY records evidence boundaries and the broader operational gaps.


## Post-publication review: gate the native encryption dependencies

Review of `114f33ddf` identified that the initial workspace-wide cipher selection
charged default builds for a default-off feature. Preserve one native SQLite
linkage per build by keeping workspace `bundled` SQLite and enabling
`rusqlite/bundled-sqlcipher-vendored-openssl` only through core's existing
`memory-prototype` feature, which CLI already forwards. Make direct `getrandom`
and `zeroize` optional; they can still occur transitively elsewhere. Keep the
standalone cipher harness and unconditional HUD export refusal unchanged.

The ordinary bundle is SQLite 3.54.1; the cipher bundle remains SQLCipher 4.19.0
with SQLite 3.53.4. Verify actual runtime versions and FTS5, absence of cipher in
default builds, and the resolved feature graph. A default-only runtime regression
must fail against the initial unconditional engine. Feature-only cipher HUD tests
remain enabled in memory builds.

The default engine changed, so rerun its complete workspace suite. The encrypted
runtime and resolved dependency graph are unchanged, permitting targeted fresh
memory/guard/native/CLI checks alongside the prior full 7,008-test feature result;
state that evidence boundary explicitly. Also run strict Clippy, MSRV and package
checks, independent fix review, exact final privacy checks, and latest-head CI and
review after pushing. Separately exercise plaintext stores in both directions
between the two actual runtime versions, including FTS5, sequence state and
committed WAL recovery. Current publication state remains on PR #846 and the Site.

**Qualification complete:** the new default suite passed 6,353/0/25; focused
feature HUD, memory and native/CLI tests, the 372-test standalone suite, 29 CLI
checks, Clippy, MSRV and packaging passed. The separate bidirectional plaintext
WAL/FTS/sequence probe also passed. HISTORY records the evidence boundaries;
PR #846 tracks final publication and exact-head CI/review.

## Follow-on boundary

Full disaster recovery preserving acknowledged forgetting requires independently durable current suppression authority, write-ahead acknowledgment, crash reconciliation and a trusted latest-generation policy. Key recovery is separately owner-controlled. Defer these and vector retrieval to their own qualified increments; later vector work reuses Mecha's existing Embedder and on-demand embedding service, with no model calls inside storage transactions.
