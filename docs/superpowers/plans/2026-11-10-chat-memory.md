# Encrypted Multi-turn CLI Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retain and recall source-linked episodes during opt-in CLI conversations with changing goals and authenticated resume.

**Architecture:** Extend the existing encrypted SessionMemory authority with a distinct chat protocol and per-turn immutable transcript prefixes. Keep context evidence separate from confirmed criteria, and preserve cumulative consulted-source lineage across the conversation. CLI owns input/rendering; core owns admission, authorization, tracking and recovery.

**Tech Stack:** Existing Rust workspace, feature-gated memory/SQLCipher, native JSONL session records. No new database or inference dependency.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.25.

## Global Constraints

- Default-off. Preserve fixed-goal one-shot protocol, historical serialization and receipt hashes.
- No model authors, suggests or edits charter lines. Owner chat is evidence, never a manufactured confirmed GoalRef.
- Chat observations are NotEvaluated; no inferred task-success verdict.
- Preserve original source pointers, cumulative consulted dependencies, current deletion/scope authority, path jail, taint and fail-closed profile checks.
- Keep SQLCipher/key/authority outside the task workspace. External JSONL remains a separate protection surface.
- First surface is local recorded CLI chat with explicit `--episodic-memory` and `--dynamic-goals`; web/TUI and automatic learning are deferred.
- No production activation, model switch, owner-graph migration or PR merge. User has authorized Spark source transfer/testing, draft PR updates and private Site updates.
- Synthetic fixtures only. Qualification owns remote Cargo; workers do not run competing builds. Root owns documentation and integration commits after required privacy scans.

## Review Focus

- Appending a later turn must not invalidate an earlier prefix; mutating that prefix must invalidate it (Task 2).
- Resumed or compacted history must not turn recalled evidence into independent corroboration (Task 2).
- A malformed/missing protocol marker, swapped store or competing writer must refuse before provider execution (Task 2).
- Context-only owner language must not gain confirmed-criterion semantics or expose raw sensor configuration in reader packets (Task 1).
- Crash/retry and failed recording must not repeat actions or silently advance the conversation chain (Task 2, actual CLI trial in Task 3).

### Task 1: Typed chat observation context

**Files:** `mecha-core/src/memory/ordinary_observation.rs`, `host_capture.rs`, `interpretation_reader.rs`, related metadata/native reader modules and affected constructors/tests. Add a small `memory/chat_observation.rs` module if useful.

**Interfaces:** Existing HostCapture gains explicitly optional confirmed criterion and an optional typed chat context, with absence omitted so legacy bytes do not change. Define `ChatObservationContext` and bounded context-version/decision locator DTOs. Chat protocol is `ordinary-chat-observation-v1`, recorder `ordinary-chat-recorder-v1`. Exactly one valid context form is accepted: existing protocols require a confirmed criterion and no chat context; chat requires chat context and no confirmed criterion. The context carries conversation/turn identity, prefix length/digest, owner message locations, sanitized historical version summaries and decision ordinals/digests. Do not serialize raw ContextVersion/CharterSnapshot into reader-visible memory.

- [x] Add regressions: legacy payload bytes/hash unchanged; chat without context, mixed criterion/context, malformed identities, reserved-label spoofing and raw-sensor exposure refused; valid chat has NotEvaluated and canonical reader roundtrip.
- [x] Obtain a failing regression result against old behavior (or compilation failure for the new API) through the qualification worker.
- [x] Implement the typed context, strict validation and reader/metadata propagation. Adapt existing constructors mechanically without changing current appraisal's required target criterion.
- [x] Run focused memory tests and formatting; review complete changed surface before Task 2.

### Task 2: Conversation authority, per-turn capture and CLI integration

**Files:** `mecha-core/src/memory_session.rs`, new focused `memory_session/chat*.rs`, `memory_session/authority.rs`, `recall.rs`, `recording.rs` as shared helpers need; `session.rs`; `mecha-cli/src/commands/chat.rs`, `run.rs`, `main.rs`; dedicated core/CLI tests.

**Interfaces:** Add host-owned chat attach/begin/finish APIs under `memory_session::chat`, using the existing SessionMemoryRequest store. An opaque attachment holds the conversation lease; an opaque turn handle owns tracked recall. Source guard dispatch distinguishes chat seals from unchanged v1 seals. Session marker is always decodable even without memory-prototype. New CLI argument `chat --episodic-memory REQUEST_JSON` requires `--dynamic-goals`, terminal input and recording. Reuse/refactor ordinary config preflight; never duplicate a relaxed profile. The Task 1 context DTO is assembled from validated native GoalContextDecision records, not model arguments.

Add a context-only chat constructor/path to `tool::goal_memory::GoalMemory`: current authenticated ToolCtx supplies owner context; no synthetic GoalRef and no required fixed `serves` value. Keep existing constructors unchanged. An unflagged loader refuses recognized markers; an explicit memory attachment requires the expected marker against its private ledger. Do not claim detection of arbitrary owner-edited transcript copies with every marker removed, and do not add implicit global store discovery. Require prior-turn retention before continuation; sealed admission retry never dispatches. Old-seal checks read only the sealed prefix, while resume checks the complete current tail.

- [x] Add regressions for two turns with different goals/charters, append-compatible prefix checks, original line pointers, strict protocol marker, wrong store/workspace, concurrent attachment and dependency withdrawal after resume.
- [x] Add interruption tests at begin, tool delivery, recording, seal and admission boundaries. Sealed retry is idempotent and dispatches no provider/tool; an ambiguous unfinished turn refuses new execution.
- [x] Implement an immutable per-turn chain with bounded prefix validation, cumulative consulted-source closure and explicit completed/recoverable/unavailable states. Use an exclusive lock for the conversation. New-source originals are only the current turn; history and prior consulted sources remain dependencies. Reject unsupported rewrites/compaction explicitly if canonical provenance cannot be recovered; do not silently accept incomplete lineage.
- [x] Add feature-independent native session marker and refuse unsupported resume paths. Integrate CLI preflight before model/router setup, then per-turn tracked tool binding and post-recording finish. Ensure marker/config/tool schema ordering agrees with replay. Default-off behavior and one-shot tests remain unchanged.
- [x] Keep slash commands safe: block new grants, and refuse clear if authenticated epoch handling is not implemented. Surface retention status without claiming success. Preserve historical taint.
- [x] Run targeted core/CLI tests, then scoped independent review. Resolve API details in the report so Task 3 can drive the real binary.

### Task 3: End-to-end qualification, documentation and publication

**Files:** bounded authored CLI regression script under `scripts/` or existing memory pilot test location; `docs/HANDOFF.md`, `HISTORY.md`, `ARCHITECTURE.md`, `MECHA-MEMORY-DESIGN.md`, `website/docs/features/memory/index.md`, `CHANGELOG.md`; existing Site source.

**Interfaces:** Use actual built `mecha chat` in a PTY and Task 2's printed capture IDs/status. All data is authored, with isolated home/workspace/authority. Preserve all failed attempts and compare reported capture with stored authenticated records and actual provider requests.

- [x] Exercise first task, changed owner goal, charter edit, restart with same store, relevant recall, historical replay and missing-flag/wrong-store refusal before requests. Include a historical distractor and verify which source packet was delivered.
- [x] Run bounded resident-model memory-on/off comparison only after transport contract passes; record choices, citations, evidence delivery and costs separately. Do not claim benefit from a tiny or unequal-evidence comparison. No model changes or production configuration edits.
- [x] Run workspace all-target build, full default and memory tests, strict Clippy, formatting and relevant strict sandbox tests on Spark; validate Docusaurus and Site links/layout.
- [ ] Root performs source-grounded handoff, updates the compact existing Site overview and detailed archive, performs strict privacy scans, commits/pushes PR 846 and publishes owner-private Site. Read exact-head CI and repository review; fix major/medium findings, retain minor notes. Do not merge or activate production memory.
