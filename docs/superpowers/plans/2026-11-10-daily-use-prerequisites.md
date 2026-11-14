# Daily-use Memory Prerequisites Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing web dynamic-goals option reachable and retain bounded metadata for oversized ordinary tool inputs.

**Architecture:** Reuse the existing web follower options and `ActionInvocation::Oversized` representation. These are two independent adapter fixes, not a new persistence layer or web-memory attachment. The user authorized autonomous progress through daily-use gaps and parallel implementation.

**Tech Stack:** Existing Rust workspace and memory feature; no new dependency or wire format.

**Spec:** The complete narrow description below, under Design contract. Broader memory architecture remains `docs/MECHA-MEMORY-DESIGN.md` §31.25; subsequent web-memory/MCP/compaction work is separate.

## Design contract

1. `mecha serve --dynamic-goals` must carry only that explicit opt-in from parsed CLI options into the web follower's stored setup options. Do not forward all global options: web global-only configuration, producer workspace, no-skills posture, owner authentication, taint and provider-follow policy remain unchanged. Default false remains false. Rebuild/rebind must retain the selected mode. Existing authenticated typed owner ingress is reused; voice, task prompts and synthesized inputs do not acquire owner provenance. This does not attach encrypted memory to web chat.
2. Both one-shot and CLI-chat memory capture must preserve an oversized tool invocation as `InputStatus::Oversized`, null `input`, digest of the full native serialized input, original line/pointer, call identity and receipt. Serialized inputs of at most 1024 bytes remain Complete with identical historical bytes. No truncation, enlarged capture budget or fabricated argument text. Keep full input internally for exact recall-attempt reconciliation and native transcript hashing. A digest is identity evidence, not inspectable content or success.
3. Existing transcript/result/passage/count limits, source mutation/withdrawal checks, NotEvaluated assessment and complete-seal retry remain. This does not add large-result support, graph tools, todo/formal planning or compaction. Those are subsequent increments after these prerequisites.

## Global Constraints

- Isolated worker checkouts; root integrates and owns docs/commits/publication. Qualification remains sole remote Cargo owner.
- Authored fixtures only. No production activation, model switch, additional real-model cohort, graph data access or PR merge.
- No new dependency, protocol enum, config field or general registry redesign.
- Preserve owner-authored charter, path jail, conversation taint, failure-closed preflight and default-off behavior.
- Test actual entry/adapters, not a direct test-only enabled Agent or only schema construction.

## Review Focus

- CLI parsing passes a flag that actual serve setup discards: Task 1 must cover the real shared option path.
- Follower rebind drops dynamic mode or accidentally forwards unrelated global authority: Task 1 verifies stored/rebuild options and web fixed defaults.
- A web turn gains owner provenance merely because its role is User: Task 1 pins typed owner vs non-owner paths.
- UTF-8/JSON escaping crosses the 1024-byte limit differently than character count: Task 2 covers exact serialized-byte boundaries.
- Omitted invocation content is quoted or mistaken for verified execution: Task 2 checks explicit status/null/full digest/source pointer/NotEvaluated and existing downstream refusal.

### Task 1: Reachable web dynamic goals

**Files:** `mecha-cli/src/main.rs`, `commands/serve/mod.rs`, `commands/serve/chat.rs`, focused CLI/follower tests as needed.

**Interfaces:** Pass an explicit bool `dynamic_goals` (or single-field typed options) from main's Serve arm to `serve::execute`, then `ChatState::build` and `web_agent_opts` before `Follower::start`. Follower stores options already; avoid post-construction mutation.

- [x] Add failing entry-point regression parsing `serve --dynamic-goals` and checking the exact options used by web setup; default-off and unrelated global flags cannot loosen web defaults.
- [x] Demonstrate RED against current wiring, then implement narrow forwarding across all callers.
- [x] Test rebuild/rebind mode preservation and authenticated typed owner context vs non-owner inputs; use existing deterministic fixtures. Preserve request binding and charter registration.
- [x] Qualification runs focused tests and CLI build; independent reviewer verifies both spec and security-sensitive boundaries. Root updates existing user docs and history after actual checks.

### Task 2: Oversized invocation capture

**Files:** `mecha-core/src/memory_session/recording.rs`, `chat_recording.rs`, focused ordinary/chat lifecycle tests. A shared pure helper in recording is appropriate if it eliminates duplicate construction.

**Interfaces:** Reuse existing `ActionInvocation`, `InputStatus` and `authority::digest`. Construct Complete for `serde_json::to_vec(input).len() <= 1024`, otherwise Oversized/null with the same full input digest. No changes to stored type or its validator.

- [x] Add adapter regressions for one-shot and chat oversized fs_write; old behavior withholds capture, new behavior retains NotEvaluated with exact original source identity.
- [x] Demonstrate RED, then implement bounded metadata construction without changing the full input call map.
- [x] Pin 1024/1025 serialized bytes, Unicode/escaped text, unchanged Complete serialization, matching receipt, digest/source pointer, source mutation and sealed retry. Existing result/capture bounds still refuse. Recall tracking remains exact and cannot bypass its own stricter input bounds.
- [x] Run focused lifecycle and relevant support/reader tests; independent review confirms omitted content does not gain factual support.

### Task 3: Integration qualification and current status

**Files:** Existing docs/HANDOFF/HISTORY/ARCHITECTURE, CHANGELOG, user appraisal/memory pages and private Site; no duplicated progress sections.

- [x] Integrate reviewed patches onto the published CLI-memory baseline with postimage hashes.
- [x] Run fmt, strict all-feature Clippy, default/memory workspace checks appropriate to shared CLI/core changes, and standalone harness compatibility. Exercise actual ordinary CLI with a larger authored write; actual authenticated web request path for dynamic context.
- [ ] Record test vs runtime evidence separately. Update compact Site roadmap, strict privacy-scan exact outgoing source/message/PR body, normal hooks and PR push. Read exact-head CI/review; fix major/medium findings. No merge or production activation.
