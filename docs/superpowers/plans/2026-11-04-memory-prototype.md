# Mecha Memory Prototype Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a runnable, synthetic-data memory prototype alongside existing mecha-graph, without moving nightly activities or changing live stores.

**Architecture:** Implement an opt-in `mecha_core::memory` module and a standalone Rust harness compiling that same source. Use a deliberately small SQLite schema for evidence, entities, grounded relations, immutable revisions, input dependencies, scopes, lexical search and source suppression. This is the first slice, not the full production schema or migration.

**Tech Stack:** Rust, rusqlite 0.40, serde, serde_json, chrono, uuid, sha2; existing dependency versions, no model/network service.

**Spec:** ../../MECHA-MEMORY-DESIGN.md

## Global Constraints

- Nightly activity migration is deferred until its value is established.
- Synthetic fixtures only; ordinary SQLite is not qualified for private data.
- No live store discovery, connector invocation, model calls, timer registration, MCP registration, or installed binary changes.
- One explicit new prototype file; reject existing unrelated databases and Mecha home paths.
- Latest revision before lifecycle filtering; occurrence time, valid time and knowledge time remain distinct.
- Current restrictions override historical snapshots. Every relation records its evidence and entity revision dependencies and inherits their scopes.
- No execution authority, policy activation or task mutation is exposed.
- Full production encryption, import, branch overlays, approximate date reasoning, model extraction and operational-store migration remain later slices.

## Review Focus

- Retry versus A→B→A edits: only equal current capture is a no-op.
- A correction or withdrawal cannot resurrect an older eligible revision.
- Filtering before the result limit must not hide an eligible record behind inaccessible records.
- Forgetting must erase transitive derived payloads and search entries, invalidate old snapshots and suppress recapture.
- An unrelated existing SQLite file must never be initialized or mutated by the prototype opener.

### Task 1: Store, captures and revision contracts

**Files:** `mecha-core/src/memory/{mod.rs,model.rs,store.rs,schema.sql,tests.rs}`, `mecha-core/src/lib.rs`, `mecha-core/Cargo.toml`, `prototypes/memory/{Cargo.toml,src/lib.rs}`.

**Interfaces:** `Memory::create(path, domain)`, `Memory::open(path)`, `capture(Capture) -> RevisionRef`, `entity(label, scopes) -> RevisionRef`, `snapshot() -> Snapshot`, `read(reference, access) -> Option<Record>`.

- [x] Write tests for explicit isolated creation, foreign-file rejection, restart persistence, idempotent capture and A→B→A history.
- [x] Run the standalone harness tests; observe missing/unimplemented operations fail.
- [x] Implement file identity checks and atomic revisions with domain-bound snapshots and source deduplication.
- [x] Run tests and commit the cohesive prototype after all tasks pass.

### Task 2: Grounded relations and time-aware retrieval

**Interfaces:** `relate(RelationInput) -> RevisionRef`, `revise_relation(expected, input)`, `withdraw(expected)`, `search(Query, access) -> Vec<Hit>`.

- [x] Add failing tests for exact source spans, invalid bindings, stale expected revisions, interval boundaries, old knowledge snapshots and no resurrection after withdrawal.
- [x] Add failing tests for scope inheritance, dependency staleness and eligible-before-limit search.
- [x] Implement native/fixture relation admission, UTC canonicalization, explicit unknown/open bounds, FTS lexical search and deterministic ordering.
- [x] Run the focused suite; all named contracts must pass.

### Task 3: Forgetting and fixture harness

**Files:** `mecha-core/src/memory/forget.rs`, `prototypes/memory/src/main.rs`, `prototypes/memory/tests/cli.rs`.

**Interfaces:** `forget(source: SourceKey) -> ForgetReport`, CLI `demo PATH`, `query PATH TEXT [KNOWN_SEQ] [VALID_AT]`, `forget PATH SOURCE_ID`, `inspect PATH`.

- [x] Add failing tests for transitive erasure, FTS cleanup, historical read blocking, persisted suppression, duplicate source version handling and denied recapture.
- [x] Implement transactional restriction/erasure with secure-delete, DELETE journal and no claim about external copies or physical media erasure.
- [x] Add a synthetic demo with dated relations and corrections, restart/query/forget commands and machine-readable output.
- [x] Run CLI integration tests proving unrelated-file safety and the demo's historical/current behavior.

### Task 4: Verification and handoff

**Files:** `prototypes/memory/README.md`, design status/addendum, docs index, `CHANGELOG.md`.

- [x] Run harness tests, formatting and clippy; inspect a real CLI demo/query/forget result.
- [x] Attempt repository-required build/tests/clippy and report unavailable dependencies honestly; do not change the production lockfile/toolchain to make them pass.
- [x] Obtain a fresh whole-change review, address consequential findings with regression tests, and rerun affected checks.
- [x] Document precise prototype limitations and commands; commit locally, no push or deployment.

## Execution ledger

- Baseline: full workspace build and offline retry cannot resolve pinned `monty-pool`; no source changes caused this. Network approval review timed out.
- Ruling: use an independent harness compiling the exact core module so the prototype can be tested with cached dependencies. This does not establish full-workspace compatibility.
- Ruling: managed worktree creation succeeded, but its path is outside writable roots. Continue on the existing clean design feature branch in the authorized checkout; leave the unused managed worktree untouched.
- Ruling: source suppression uses a domain-salted digest in this synthetic prototype. Production needs the designed keyed suppression and encryption qualification; no private import is exposed.
- Tasks 1–3: implemented. Initial stub suite failed all ten core contracts; implementation passed them. CLI demo failed before implementation, then passed its lifecycle test. Compact schema is in `schema.sql`; query/validation helpers were split into `sql.rs` for clarity.
- Review: indirect old-self dependency reproduced by a failing regression; admission now rejects the entire transitive object closure before commit. Independent reviewer verified the fix. Added entity/consulted scope and concurrent optimistic-write coverage.
- Ruling: lookup of a forgotten revision returns `None` in this slice. Distinguishing missing/forgotten/unauthorized and persisted retrieval receipts remain explicit production gaps.
- Handoff review: added only this prototype's remaining qualification gates to HANDOFF; no installed-system claims or unrelated historical items were changed.

- Final verification: 13 core + 2 CLI tests passed; harness clippy with warnings denied and repository/harness formatting checks passed. Full-workspace build/test/clippy remain blocked by missing pinned monty-pool. Demo returned May/current=1, July/current=0, July/old-snapshot=1, July/old-snapshot-after-forget=0.
- Managed-worktree cleanup was refused because the app treats it as the primary checkout; it remains unchanged. All prototype changes are in /Users/lukechang/Github/mecha on codex/mecha-memory-design.
