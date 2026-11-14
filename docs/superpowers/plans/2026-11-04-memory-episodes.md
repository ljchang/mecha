# Memory episode capture and baseline Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline, then a fresh whole-change review. Execution authorized in the chat.

**Goal:** Import synthetic native Mecha JSONL sessions into the existing memory prototype and measure source-grounded lexical retrieval against oracle evidence.

**Architecture:** Keep exact physical JSONL records as versioned evidence with optional transcript metadata. Index readable message text separately from raw JSON. Reuse existing revisions, scope checks, suppression and FTS; implement in core with thin CLI commands.

**Tech Stack:** Existing Rust, rusqlite/FTS5, serde_json; no new dependencies.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md`, `docs/AGENT-MEMORY-SYSTEMS-RESEARCH.md`, and the approved four-step next-implementation description in chat.

## Global Constraints

- Default-off `memory-prototype`; synthetic files only, no live discovery or nightly migration.
- Preserve raw records including unknown records/blocks; never interpret rewrite summaries as original utterances.
- Native message records generally have no timestamps: keep unknown, do not substitute session creation time.
- One explicit session file per import; append-only prefix contract; malformed/truncated/changed prefixes fail atomically.
- Current source restrictions and suppression apply to historical reads and expanded context.
- Oracle evidence measures retrieval opportunity, not answer quality; no model evaluation claimed.

## Review Focus

- Re-import after append, changed/truncated prefixes, and interrupted final lines must not duplicate or partially commit.
- Future record kinds/blocks retain raw data but never gain invented speaker/time or instruction authority.
- Context windows must not cross accounts/sessions or leak restricted neighbors; limits follow eligibility.
- Forgotten source records remain absent from FTS, historical context and repeated imports.
- Evaluator gold labels must never influence baseline retrieval, and misses must remain visible.

### Task 1: Session adapter

**Files:** `mecha-core/src/memory/{model,store,sql,mod,transcript,transcript_tests}.rs` and existing capture constructors.
**Interfaces:** `Memory::import_session_jsonl(reader: impl Read, account: &str, scopes: BTreeSet<String>) -> Result<ImportReport>`; transcript metadata on `Capture`; stable `transcript_source(account, session, line) -> SourceKey`.

- [x] Add failing tests for exact retention, retries/appends, atomic malformed input, prefix changes/truncation, missing timestamps, unknown records, scope updates and forgotten-source suppression.
- [x] Implement bounded parsing (32 MiB file, 1 MiB record), native `meta` identity, text-field JSON pointers and transactional import using a shared capture helper.
- [x] Run `cargo test -p mecha-core --features memory-prototype memory::` and standalone prototype tests.

### Task 2: Evidence retrieval

**Files:** `mecha-core/src/memory/{recall,recall_tests,mod}.rs`.
**Interfaces:** `Memory::recall(&RecallQuery, &Access) -> Result<Vec<RecallHit>>`; query includes snapshot, optional account/session and occurrence range, result/window/character limits; hits contain exact revision-backed anchors and bounded context.

- [x] Add failing tests for BM25 ranking, scope filtering before result limit, source/time filters, unknown dates, window isolation, historical restrictions, forget and UTF-8 budget bounds.
- [x] Implement evidence-only BM25 retrieval and window expansion within one read transaction; report source citations and conservative taint labels. No raw config/rewrite indexing.
- [x] Run focused tests; inspect query plans and deterministic ordering.

### Task 3: Evaluation and CLI

**Files:** `mecha-core/src/memory/{evaluation,evaluation_tests}.rs`, synthetic fixtures under `prototypes/memory/fixtures/`, `mecha-cli/src/commands/memory.rs`, `mecha-cli/tests/memory_cli.rs`.
**Interfaces:** `evaluation::run(path: &Path) -> Result<EvaluationReport>` creates a fresh synthetic store. CLI adds `init`, `import-session`, `recall`, `evaluate`; import paths are explicit operator inputs, never tool inputs.

- [x] Add failing integration tests for native JSONL round-trip, required synthetic acknowledgement, structured output, source-specific forgetting, and evaluation reports with visible baseline misses and complete oracle coverage.
- [x] Build 32 fixed cases covering direct recall, corrections, chronology, indirect relevance and changed goals; expected sources are held apart from recall inputs. Report per-case evidence sets/coverage and timings, with reasoning explicitly unmeasured.
- [x] Keep CLI thin; add usage, limitations and design/prototype addenda.
- [x] Run fmt, clippy, default and all-feature workspace tests, standalone tests and black-box smoke commands. Fresh reviewer checks entire diff, then fix substantive findings and run handoff.

## Execution ledger

- Pre-flight: adapter metadata is consumed by recall and evaluation; all use the existing `Capture`/`RevisionRef` and current access policy. No schema version change is needed for optional payload metadata.
- Baseline workspace build passed; macOS linker emitted its existing large unwind-section warning. Default workspace tests started before edits.
- Baseline default workspace tests passed with local socket permissions; sandbox-only run failed on denied test sockets.
- Tasks 1–2: tests first failed on absent adapter/recall interfaces, then all 22 core harness tests plus two harness CLI tests passed. Historical reads retain the existing requirement for both historical and current scopes.
- Task 3: new main-CLI tests failed on absent commands, then all five main-CLI tests passed. Evaluation tests passed with visible misses. All-feature Clippy passed with warnings denied.
- Implementation choice: full evaluation JSON lives under the core module (rather than outside the published crate); a small operator example remains under prototypes/memory/fixtures. This keeps crate packaging self-contained.
- Final review found one P2: global BM25 statistics allowed inaccessible future records to change historical top-k results. The focused regression failed before the fix and passed after eligible-corpus scoring.
- Ruling: use a disposable in-memory FTS scoring index over eligible evidence for this baseline. Persistent FTS statistics cannot be filtered with an SQL WHERE clause. Cost: corpus-sized query work; large-corpus performance must be qualified before private/live use.
- Review boundaries accepted: production security/migration, model reasoning quality and large-corpus performance remain explicitly unqualified; full workspace verification stays with the implementer. Research prose is separate from this implementation review.
- Additional source-fidelity regression: future record kinds carrying numeric timestamps failed before the adapter stopped interpreting unknown fields, then passed. Native message timestamps still require RFC3339 when explicitly supplied.
- A post-review workspace run encountered the existing `situation_brief::a_web_run_records_every_field_and_sends_none` bind-port race (`Address already in use`). All seven tests passed serially; full-suite verification was repeated with reduced test concurrency.

- Final verification: 5,920 all-feature workspace tests passed, 25 ignored; default workspace 5,891 passed, 25 ignored; standalone 24 core + 2 CLI passed; fmt and both Clippy runs passed with warnings denied. Final full run used normal test concurrency. The unrelated watchdog startup test also passed in isolation after one full-run timing failure.
