# Dynamic goal context implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Let owner chat instructions and owner-authored charter revisions change a conversation's goal context while preserving exactly which version each decision used.

**Architecture:** Keep immutable owner evidence separate from interpreted objectives and the existing confirmed GoalRef anchor. Trusted conversation entry points supply owner input; core versions it, exposes it to planning/appraisal, and records its effective decision boundaries. Replay consumes recorded versions, never today's charter.

**Tech Stack:** Existing Rust core, JSONL session records, conversation runtime, scripted-provider tests; no new database or inference service.

**Spec:** The owner's approved dynamic-goal requirements in this conversation; existing authority constraints in `docs/ARCHITECTURE.md` sections The goal system, Session records and replay, Interruption and steering, and Experimental memory prototype.

## Global Constraints

- Charter bytes are owner-authored. No model tool or inference can compose, suggest, or write charter lines.
- Natural owner chat is sufficient source input; do not require a fabricated task ID, a goal command, or confirmation of every turn.
- Chat instructions accumulate as versioned evidence. Do not automatically treat every sentence as a replacement goal or a new standing priority. Model interpretations do not acquire owner authority.
- Preserve exact shown charter content and owner input, revision identity, predecessor, and effective boundary. A hash without retrievable content is insufficient.
- Reuse GoalRef/GoalTrack for their existing confirmed-pointer/drift semantics; do not relabel model guesses as confirmed references.
- New context is conversation-local and survives resume and compaction. Queued but undelivered steering must not become active.
- Tool output, retrieved content, model prose, unattended prompts, and unverified remote messages are not authenticated owner instructions.
- Missing or corrupt new-format replay context must refuse the affected comparison; never substitute current state. Legacy transcripts keep explicit legacy behavior.
- Ordinary memory v1's fixed-goal capture remains unchanged. Incompatible new traces must be rejected before capture claims, not silently admitted or downgraded.
- Preserve incognito/no-session behavior, security taint, grants and provider/tool independence. No production activation or merge.
- Activate through an explicit `--dynamic-goals` startup option; once enabled, natural owner chat updates context automatically. Existing sessions do not silently acquire new recording bounds. A versioned session resumed through an unsupported legacy path refuses rather than dropping context.

## Review Focus

- Owner changes focus versus adds a subordinate task; earlier instructions remain attributable without becoming an automatic contradictory goal stack.
- Charter edited, deleted or malformed after recording; historical replay remains unchanged and current malformed policy does not silently vanish.
- Compaction, clear, cancellation, retry and resume cannot relabel past decisions with the latest revision.
- Two concurrent conversations on one Agent cannot share goal context or mutable tool bindings.
- Mid-run steering, replay divergence, partial records, and front-end authority differences must be supported explicitly or refused visibly.

### Task 1: Versioned context, durable records and replay

**Files:** new focused core goal-context module; `session.rs`, replay modules, `lib.rs`; focused unit/integration tests.

**Interfaces:** Define validated immutable context and owner-source types, a host-only transition API, and effective-boundary records. Publish the exact API contract to Task 2 before it implements callers. Session hydration restores versions and preserves historical associations across rewrites. Replay reconstructs recorded context for turns/branches and refuses unsupported or ambiguous traces.

- [x] Write meaningful failing tests for owner revisions, charter identity/content, corrupt chains, historical lookup, compaction/resume and legacy compatibility.
- [x] Implement bounded source-preserving context without a semantic classifier or new authority-bearing model tool.
- [x] Implement transcript recording/hydration and replay restoration; test historical charter changes and unsupported steering/divergence explicitly.
- [x] Review Task 1; fix verified medium/major findings and regression-test them.

### Task 2: Runtime and trusted conversation entry points

**Files:** `agent.rs`, tool context and `goal_context`, setup, CLI chat, TUI and authenticated assistant web chat; focused tests. Do not edit Task 1 files without coordination.

**Interfaces:** Consume Task 1's typed context through conversation-local state. Capture owner input before message folding; refresh owner charter at safe run boundaries; bind exact context to actual provider requests and goal_context results. Existing autonomous and persona flows must not accidentally opt into owner authority. Coordinate runtime/replay interface changes with Task 1.

- [x] Write scripted-provider tests for two successive goals, charter refresh, concurrent conversations, failed persistence and untrusted input exclusion.
- [x] Wire natural chat through explicit trusted ingress; preserve owner text without pretending a model interpretation is confirmed.
- [x] Expose current version to planning/appraisal consumers and actual provider requests, preserving original GoalTrack semantics.
- [x] Integrate replay and ordinary fixed-goal compatibility tests with Task 1; verify clear/resume/steering behavior at each enabled surface.
- [x] Review Task 2; fix verified medium/major findings and regression-test them.

### Task 3: Qualification and publication

- [x] Establish clean baseline on the dedicated remote checkout, with one Cargo writer.
- [x] Qualify combined diff with workspace build/tests, formatting, strict all-feature Clippy, memory-feature compatibility and focused replay/provider-request tests.
- [x] Independent whole-change review and bounded fix/re-review; preserve failing attempts as evidence.
- [x] Update existing design/invariants, user docs, changelog, verified handoff and compact Site roadmap.
- [ ] Strict source and exact commit-message privacy scans; commit/push to draft PR #846, inspect exact-head CI and automated review, publish Site with unchanged audience.
