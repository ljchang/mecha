# Read-only memory context implementation plan

> **For agentic workers:** Use superpowers:executing-plans. The user has authorized implementation in the existing isolated worktree.

**Goal:** Supply bounded, source-linked prototype memory to an existing planning workflow without changing default behavior.

**Architecture:** A core context API combines eligible episode recall, up to two exact-source-quote follow-up searches, and relations supported by retrieved episodes. An opt-in replacement for `goal_context` adds this packet on demand in `mecha run`; the original tool remains untouched when off. The adapter holds a read-only SQLite connection, host-owned access/snapshot/filters and situation; it has no write or provider interface.

**Tech Stack:** Existing Rust, SQLite/FTS5, serde, tokio; no new dependencies.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md`, prototype invariants in `docs/ARCHITECTURE.md`, and the user's approved read-only context increment.

## Global constraints

- Default-off `memory-prototype`, synthetic data only; no production database migration, nightly jobs, or graph replacement.
- Exact quote from an original visible source field; no composed name or metadata binding. Host derives the mention, never broad-search fallback after invalid binding.
- Current authorization and forgetting override snapshots. Relations require all dependency evidence to satisfy account/session/time restrictions.
- Bounded serialized packet, original provenance, unknown timestamps preserved; retrieval is not truth/confidence or proof of completeness.
- Retrieved content is private and external, served on demand, never trusted prefix/learning. Numeric charter/homeostat sensors stay internal.
- Existing benchmark protocols remain frozen. CLI integration only in `run`; no implicit inheritance into subagents/other surfaces.

## Review focus

- Revocation, forgetting, and source updates between retrieval stages must not leave stale/disallowed evidence in the returned packet.
- Relation dependencies outside the selected account/time scope must not leak through an otherwise eligible support episode.
- UTF-8, fabricated/noncontiguous quotes, unknown fields and oversized requests must fail without broad fallback.
- Registry replacement must respect a withheld `goal_context` and preserve interlock arming, including code-tool rebuild.
- Budget omission/empty evidence must not be interpreted as negative evidence; errors must remain harness-originated.

### Task 1 — Core packet and source selection

Files: `memory/context.rs`, `memory/context_tests.rs`, `memory/store.rs`, `memory/mod.rs`.
- [x] Write failing tests for exact-quote follow-up recovering a fact, fabricated quote refusal, read-only writes rejected, account/time/provenance/forgetting and whole-packet budget.
- [x] Implement `Memory::open_read_only`, `Memory::context(request, policy)` and typed request/policy/packet with atomic final eligibility checks.
- [x] Run feature memory tests and standalone prototype tests.

### Task 2 — Existing planning tool adapter

Files: `tool/goal_memory.rs`, `tool/mod.rs`, `mecha-cli/src/commands/run.rs`.
- [x] Write failing tests for opt-in schema, off behavior, exact selected quote routed end to end, external/private labels and local-error labels; CLI opt-in prerequisites.
- [x] Add a host-bound read-only service and `goal_context` wrapper; replace only an already-available tool for explicit `run` options, before session config is recorded.
- [x] Run targeted tests, then fmt, workspace all-target build, strict all-feature clippy and workspace tests.

### Task 3 — Review and handoff

- [x] One independent final review (executing-plans skill), fix important findings with regression tests.
- [x] Document usage and limits in existing prototype README, design addendum, architecture, changelog, handoff/history.
- [x] Verify final commands and local synthetic smoke. Keep worktree unmerged; Spark deployment only if it adds evidence beyond local deterministic checks.

## Execution ledger

- Baseline: build --workspace --all-targets and test --workspace passed (logs in /tmp/mecha-context-baseline-*).
- Scope: runtime exact-quote protocol is separate from frozen automatic-binding experiment; caller supplies a quote selected from original text, host derives its byte range.

- Core context tests: 3/3; standalone: 51 unit + 2 CLI; tool adapter and CLI argument tests RED→GREEN.
- Added the old automatic-binding path's missing valid non-null routing regression: same query misses palisade:4 unbound and retrieves it bound. Protocol/fixtures unchanged.
- Full suite caught the new pure-read implementation missing from replay_run::PINNED; verified it owns a read-only connection and RAM scorer, and added its PURE classification. Interlock integration passed.

- Final review at 864c1ece5: no Critical/Important findings. Deferred minor diagnostics: recall-stage budget omission flag; nested source unknown-field strictness.
- Complete all-feature rerun: 5,957 passed / 25 ignored. Strict lint and standalone checks passed. Real CLI with isolated synthetic DB and scripted streaming provider: 3 requests, follow-up recovered fact. Default workspace all-target build and default-off CLI regression passed.
