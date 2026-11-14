# Mecha Memory CLI Integration Plan

> **For agentic workers:** Use superpowers:executing-plans; implement inline and obtain a fresh final review.

**Goal:** Run the synthetic memory prototype through the actual `mecha` binary while preserving explicit isolation and deferred nightly migration.

**Architecture:** Add a thin Clap command behind the CLI's default-off `memory-prototype` Cargo feature, forwarded to core. Move synthetic fixture construction from the standalone harness to core so both entry points use identical operations. No model, config, graph connector or scheduler is needed.

**Tech Stack:** Existing Rust, Clap, serde_json and core SQLite implementation; unchanged pinned dependency versions.

**Spec:** ../../MECHA-MEMORY-DESIGN.md and ../../../prototypes/memory/README.md; owner approved CLI integration on 2026-11-04.

## Global Constraints

- Synthetic data only; no private import or encryption claims.
- Require `--experimental` and explicit `--db PATH`; core file identity and creation checks remain authoritative.
- Compile-time opt-in only: no runtime Feature registry, onboarding, tool or web surface is added.
- No model/router hold, config load or background jobs. Existing `mecha kg` remains unchanged.
- No production binary replacement, push or merge. Owner subsequently authorized
  Spark testing/deployment over Tailscale; stage in an isolated checkout because
  Spark's main and installed binary contain newer work.

## Review Focus

- Missing opt-in/database arguments must fail before filesystem effects.
- Normal builds must not expose the prototype command.
- Broken global/project config must not block fixture operations or cause router activity.
- Main CLI and standalone harness must share fixture behavior and preserve temporal/forget semantics.
- Existing files and live Mecha directories must remain protected by core validation.

### Task 1: Main-binary contract tests

**Files:** `mecha-cli/tests/memory_cli.rs`.
**Interfaces:** `mecha memory --experimental --db PATH {demo,inspect,query,forget}`; query takes text, `--known-at N`, `--valid-at RFC3339`, `--include-possible`, `--limit N`.

- [x] Add black-box tests of the actual binary: missing flags, foreign file, malformed config, historical/current queries, forgetting and normal-build absence.
- [x] Run against the pre-integration binary and observe the unrecognized-command failure.

### Task 2: Shared fixture and thin command

**Files:** `mecha-core/src/memory/fixture.rs`, `mecha-core/src/memory/mod.rs`, `prototypes/memory/src/main.rs`, `mecha-cli/src/commands/memory.rs`, `mecha-cli/src/commands/mod.rs`, `mecha-cli/src/main.rs`, `mecha-cli/Cargo.toml`.
**Interfaces:** core `fixture::demo(&Path) -> Result<Value>`, `fixture::access() -> Access`, `fixture::source_key(&str) -> SourceKey`; CLI `memory::execute(Args) -> Result<()>`.

- [x] Move existing fixture logic intact to core and reuse it from both frontends.
- [x] Implement Clap parsing, feature forwarding, dispatch and explicit non-model command classification.
- [x] Run main CLI integration and all existing prototype tests; add command-classification assertions.

### Task 3: Qualification and documentation

**Files:** prototype README, CLI reference, changelog, scoped handoff/design status.

- [x] Run workspace build/test, all-feature clippy, format and opt-in/default CLI integration tests. Record pre-existing/environment failures distinctly.
- [x] Obtain fresh read-only review; fix consequential findings with regressions.
- [x] Document actual main-binary commands and remaining private-import gates; commit locally.

## Execution ledger

- Worktree now writable; `codex/mecha-memory-cli` starts at prototype commit `fa8779b60`.
- Pinned dependency fetch succeeded without changing manifests or lockfile versions, resolving the former missing-Monty blocker.

- Task 1 RED: all three opt-in binary tests failed with unrecognized `memory` before dispatch existed.
- Task 2 GREEN: all three real-binary tests and all 15 standalone harness tests pass; shared fixture moved intact.
- Default-off feature forwarding, model/config/router exclusions and temporal/forget semantics reviewed by a fresh Codex reviewer; no medium/high findings.
- minor (deferred): query help says “grounded relations”; without `--valid-at`, matching evidence/entities are also returned (documented in README).
- Workspace all-target/all-feature Clippy with `-D warnings` and both format checks pass. Final all-feature workspace tests passed (exit 0); normal-build command absence also passed.
- User authorized Spark deployment/testing and Codex review in place of Claude review. Tailscale SSH verified; no GitHub PR opened, so Claude automation was not invoked.

- Spark isolated test deployment: `2b00d5f62`; three main-CLI and 15 harness tests pass on aarch64 Linux. Installed executable hash matches tested build; historical/current queries verified. Live binary remains `fec8d55c7`, service active, both checkouts clean.
- Final workspace suite: 5,907 passed, 25 ignored, zero failed. Implementation committed; no public push/PR or main-branch merge.
