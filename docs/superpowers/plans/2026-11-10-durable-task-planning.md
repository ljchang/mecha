# Durable Task Planning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** An explicitly selected CLI task chat updates the existing todo plan in encrypted Mecha memory and restores it safely after restart, without graph writes.

**Architecture:** Preserve the existing Task store and TodoTool machinery. Add a concrete authenticated native writer, typed Plan revisions with pending audit receipts, staged todo effects, and a versioned task-chat adapter. Complete the actual CLI workflow before presenting the storage pieces as an integrated feature; the owned web adapter follows this increment.

**Tech Stack:** Rust workspace, existing SQLCipher/rusqlite, serde native JSONL, existing Agent/CLI and scripted HTTP fixtures. No new dependencies or database engine.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.27, with existing session/goal/security invariants in `docs/ARCHITECTURE.md`.

## Global Constraints

- Work on the isolated current PR worktree; baseline `93dd245c240f1cf8a0f568a32dbb3512b332ca2c`. Preserve unrelated work. Root alone edits HANDOFF/HISTORY, stages commits, performs required privacy scans and pushes.
- One owner-created task per authenticated native session. No model task creation, owner-intent rewrite, closure or cancellation. No graph writes, graph fallback, automatic migration, production activation or merge.
- Explicit keyed schema v9-to-v10 migration; old schema/wire/profile behavior stays intact. Generic Access cannot expose Task/Plan or their private dependency closure. Historical restores have no live operational authority.
- Maximum 32 items, 512 UTF-8 bytes per content/expect/check field, 256 check declarations, 32 KiB canonical snapshot; reject duplicate trimmed content and overflow without evicting checks. Persist open and frozen declarations, never process hashes.
- Host occurrences, not provider IDs, identify writes. Reject duplicate IDs within a batch before writes. Execute task-profile calls in authored order; generic dispatch remains unchanged.
- No shell/check execution, escalation, MCP, programs, deferred jobs, steering or compaction in the first task profile. Declared checks report Skipped. Model completion is not owner closure or success.
- DB commit, durable native audit and episodic retention are distinct states. Storage-only repair never reexecutes a provider/tool or invents missing records.
- Structured todo input/result/digests/feedback are validated then omitted from generic episodic projection. Exact typed Plan/native audit retains them. Ordinary source pointers, call limits, harness-note attribution and taint remain exact.
- Use authored fixtures only. Focused tests first; root coordinates the full workspace, strict Clippy, fmt, actual CLI and required Spark privacy checks. Source transfer/testing, PR/Site publication and subagents are already authorized.

## Review Focus

1. Two todo calls in one batch: serial Plan heads and ordered feedback, no duplicate process effects or fabricated work span.
2. Crash after DB commit and before native acknowledgment: visible pending state; exact storage-only repair only when complete native evidence exists.
3. Source deleted, replaced, appended externally or modified at equal length: refuse without recreating or blessing it.
4. Dropped/re-added open or frozen check across process restart: exact declaration survives and reworded steps do not inherit verification.
5. Excluded task content: generic captures/recall cannot accidentally become a second Task/Plan content store, while omitted calls still count toward limits and note validation.

### Task 1: Authenticated native append and sync boundary

**Files:** modify `mecha-core/src/session.rs`, `mecha-core/src/memory_experience_bridge.rs`, `mecha-core/src/memory_session.rs`; create `mecha-core/src/memory_session/native_writer.rs` and scoped tests.

**Interfaces:**
- Preserve `Session { meta, path }`, public legacy APIs, native record ordering and old serialized bytes.
- Factor crate-private `encode_record_line(record: &Record) -> Result<Vec<u8>>` and one shared run-message/metadata emitter consumed by legacy `record_run` and the new writer. The emitter takes the append/proof callbacks needed for existing append/Extend/Rewrite selection; legacy program persistence remains after it.
- Add concrete crate-private `BoundNativeSession`, constructed from a trusted `Session` and expected native identity/prefix. It holds an authenticated no-follow append descriptor, expected endpoint/rolling digest, and poisoned state. Its append and run-record methods verify identity/endpoint; `sync_verified` returns an opaque `SyncedNativePrefix` containing verified bounded bytes and identity, never a public deserializable execution capability.
- Expose a component-wise no-follow append opener alongside the existing read-only helper, with no create flag. Parent directory durability must be covered. Host lifecycle/lease/record-phase restrictions are layered by Task 4; no ordinary caller is silently switched to this writer.

- [x] Add behavioral tests for append/run-record parity with legacy encoding, recognized current-tail Extend, and sync-backed exact prefix.
- [x] Add failure tests for deletion, replacement/symlinks/hardlinks, foreign growth, equal-length mutation, partial write poisoning and sync failure; no file recreation, no successful proof after failure.
- [x] Implement the concrete writer/shared encoding with bounded positional reads and descriptor identity verification; preserve all legacy transition/program paths.
- [x] Run scoped session/native-writer tests and default-feature compatibility controls; distinguish compile scaffolding from observed behavioral RED.
- [x] Independent task review verifies source and tests before downstream use. Root commits after required publication checks.

### Task 2: Typed Plan revisions and audit receipts

**Files:** create `memory/plan.rs`, `plan_schema.sql`, `plan_tests.rs`; update `memory/{mod,model,sql,task,forget,recovery,recovery_inventory}.rs` and exhaustive record/version consumers, including the standalone harness.

**Interfaces:**
- Define strict `PlanSnapshot` (ordered todo-compatible steps, optional proposed serves, BTreeMap check text/frozen declarations), `PlanOccurrence { turn_id, ordinal }`, and `PlanRevision` containing exact TaskRef, snapshot, captured numeric goal version plus decision/version digests, input/result evidence and ordered planning feedback contribution. Keep native DTOs feature-independent where required; storage authority remains non-deserializable.
- Check declarations contain nonempty check text and frozen status. Dropping an item preserves its declaration; explicitly withdrawing a check while its item remains open preserves the existing TodoTool behavior (remove that open declaration). Frozen declarations cannot be removed or changed. No capacity eviction.
- Preserve standalone memory compilation: Plan steps/declarations are strict storage DTOs. Ordered existing `planning::Feedback` is retained as bounded duplicate-free JSON text plus digest in an opaque evidence wrapper. Memory validates syntax/bounds/digest and preserves exact bytes; it never canonicalizes via `Value`. On load that wrapper is unverified runtime evidence. The concrete runtime adapter must parse and exactly round-trip through `serde_json::from_str/to_string::<Feedback>` and enforce task-profile semantics before commit, comparison or read-side use. No `to_value/from_value` path for f32 feedback, no import of the whole planner into standalone memory.
- `Memory::enable_session_plans(&mut self) -> Result<()>` explicitly upgrades v9 to v10.
- `Memory::plan_read(&self, session: &TaskSession, task: &TaskRef) -> Result<Option<PlanView>>` distinguishes a never-created plan from denied/unavailable state.
- `Memory::plan_commit(&mut self, session: &TaskSession, permit: &PlanTurnPermit, task: &TaskRef, expected: Option<&RevisionRef>, request: PlanCommitRequest) -> Result<PlanCommitResult>` validates exact current heads, typed bounds, task status and occurrence. Result distinguishes a new commit from exact replay. Store the Plan revision and pending receipt atomically. Permit one Plan object per Task; no duplicate mutable content/head table.
- `Memory::plan_pending(...)` and `plan_reconcile_audit(...)` expose bounded typed pending evidence and compare exact verified native occurrences against canonical receipt evidence. A supplied receipt ID is never sufficient. Store only opaque/digest/status bookkeeping in the audit ledger; content belongs in revision payloads.
- `PlanTurnPermit` is non-deserializable, crate-private and minted only by the authenticated task-chat host. Bind store instance, attachment digest, exact TaskRef, session/account/access and turn ID; its shared mutex-protected liveness is held through commit and revoked on finish/drop. The concrete controller validates the held lease before each operation. Serialized turn IDs alone grant no pending-sibling allowance. `VerifiedPlanAudit` is a separate opaque value constructed by the host only from authenticated synced bytes and exact occurrence validation; recovery can construct this evidence but never a live permit. Storage methods are crate-private host primitives, not a new public tool-facing API.
- Exact receipt retry rechecks current authority; changed payload/expected head refuses. Later pending commits require the same live turn; restart must reconcile before acquiring execution authority.

- [x] Test explicit/atomic migration, unchanged v8/v9 behavior, encrypted snapshot version/inventory and historical restore refusal for v10.
- [x] Test create/update/drop/reopen, two-handle CAS, exact retry/conflicting retry, wrong account/session/domain/scopes, closed/forgotten task, malformed/corrupt ledger and canonical payload validation.
- [x] Test task forgetting purges dependent Plan text and scrubbed receipts; generic read/search/recall including indirect closure remains unavailable.
- [x] Test failures after append/before receipt/before commit roll back, and pending-to-audited reconciliation accepts only exact evidence with no current-head substitution.
- [x] Run scoped memory/standalone tests and independent review. Do not claim integrated todos at this gate.

### Task 3: Staged TodoTool and ordered native result evidence

**Files:** `tool/todo.rs`, new concrete `tool/todo_memory.rs`, `tool/mod.rs`, `planning.rs`, `message.rs`, `agent.rs`, provider metadata-exclusion tests; a narrow `memory_session::chat::task_plan` child module for concrete host capabilities and its parent module declaration. Full V4 attachment/CLI wiring remains Task 4.

**Interfaces:**
- Concrete `TodoTool` memory configuration/restore reuses existing parsing, Tracked transitions and rendering. Preserve exact canonical native input JSON in the stored evidence; normalization/correction belongs to the snapshot, never a replacement for the original invocation. Validate loaded steps/serves through existing parser semantics without repairing stored corruption, and validate opaque Feedback by exact typed text round-trip/profile checks at every runtime crossing. Install a staged snapshot only after a new DB commit; replay never republishes counters/feedback. Rebuild check hashes and reset process-local work state on DB restore; never call transcript rehydrate for this variant.
- Place concrete PlanBinding/LivePlanTurn in the private chat child module so only authenticated host constructors can see Lease/attachment internals and reuse SessionMemory::memory() for keyed source-guarded reopen. The binding holds a weak lease reference; host retains separate revocation ownership. Stage under the tracker lock, release transient goal/verified locks, validate lease/source, then acquire permit liveness and SQLite. Normal finish waits for Agent publication before revocation; permit alone only covers DB commit.
- Durable TodoTool reports non-read-only state mutation and private-data access; preserve existing approval/phase/taint gates and legacy volatile classifications. Do not auto-allow it by special-casing the tool name in the loop. The authored CLI fixture may explicitly allow its closed tool surface; user documentation must explain ordinary approval policy.
- Private per-turn capability and per-call occurrence on ToolCtx carry trusted bound store/task context, captured goal version and actual dispatch result cap. No model/browser fields choose task, store or operation.
- Feature-independent native `PlanResultAudit` metadata contains bounded locators, occurrence and opaque revision/receipt IDs. It carries no key, path or execution handle. Preserve old Message serialization when absent and exclude metadata from every provider encoder.
- Task-profile dispatch runs approved calls serially in original order, rejects duplicate batch IDs before effects, and transfers each receipt plus its feedback contribution to the original result message before early exits. Validate flattened planning feedback equals ordered contributions. Other profiles retain existing dispatch.

- [x] Add tests for two todo writes in one batch, first success/second failed commit, rejected call then success, and same provider ID across separate batches. Within-batch duplicate IDs refuse before DB writes.
- [x] Stage all progress effects: tracker/freezes, counts, projected goal counters, verified removals and feedback. Same-batch bookkeeping must not count the preceding batch's last trace twice.
- [x] Before DB commit reject canonical echo exceeding the dispatch cap; assert later jobs rendering does not change successful audited bytes.
- [x] Test declared checks Skipped, no shell/escalation, owner closure impossible, tamper/frozen/open checks across restart, no double publish on retry, and raw sensor/audit sentinels absent from provider output.
- [x] Scoped runtime/default controls and independent task review.

### Task 4: Versioned CLI task chat and storage-only recovery

**Files:** `memory_session/{chat,chat_recording,recording}.rs`, new task-chat host module as needed, `memory/{chat_observation,ordinary_observation}.rs`, `session.rs` marker recognition, `mecha-cli/src/{setup,commands/chat,commands/memory_profile,commands/memory_session}.rs`, CLI argument definitions and user docs.

**Interfaces:**
- Create with `chat --episodic-memory REQUEST_JSON --dynamic-goals --memory-task TASK_JSON`; TASK_JSON is a strict owner-provided TaskDraft file, read/frozen once before execution. Resume with `--resume SESSION --memory-task-resume`; resume cannot replace intent/TaskRef. Reject graph selection and unsupported profile combinations before provider setup.
- Expose `memory-session --request REQUEST_JSON enable-planning` as an explicit owner operation: upgrade existing ordinary v8 to Task v9, then Plan v10; v9 resumes at the second migration and v10 is an idempotent no-op. Each keyed step is atomic; on a later failure report the actual surviving version. Historical restore, wrong-key and unsupported versions refuse. Init/open/attach never migrate implicitly.
- New CLI attachment `ordinary-chat-memory-v4`, explicit task observation/recorder pair; preserve old CLI/web protocol bytes. The private attachment stores exact TaskRef and selected surface, never a key. Owner-authored source establishes task intent; model serves remains a proposal.
- Owner TaskDraft establishes the task container and displays through authorized host reads; normal chat owner input remains the agent-facing goal evidence. Do not silently inject draft text as chat speech, charter or a new goal authority.
- Before Task DB creation, durably write a strict private startup claim (exact Meta identity/prefix, selected surface, manifest/store/session, draft digest) and a feature-independent native task-start reservation recognized by ordinary execution-refusal readers. This prevents a later ordinary resume from treating a reserved task session as plain Meta-only history; it is not a lock against already-running ordinary writers. Final attachment/marker derives deterministically from this claim and the original Task receipt. Expose `memory-session --request REQUEST_JSON retry-task-start SESSION --task TASK_JSON`; match the exact frozen draft and source, create/reuse only that deterministic operation, never execute a model/tool. Torn/altered reservation/marker remains blocked and reported, with no truncation or synthesized completion.
- `SessionMemory::attach_task_chat` acquires the existing lease, authenticates source/store, creates or restores the task/plan and constructs bound writer/controller. Setup wires the actual same Arc into registry and Prepared.todo. Pending work is displayed before refusal; DB failure never falls back to native echo or graph.
- Task-specific GoalIngress recording routes only decision records through its active bound writer; old with_goal_ingress callers remain unchanged. Invalidate turn capabilities on finish/drop and do not propagate them to subagents or synthetic harness calls.
- Before appending the final run records, collect their canonical encoding in bounded memory and durably prepare a private expected source identity/end/digest bound to the attachment and frozen turn. This stores fingerprints only, not a second native transcript. A failed preparation prevents final append. Recovery compares the existing complete suffix to this expectation, including zero-todo turns; unprepared, partial or mismatched suffixes remain blocked. This expectation precedes native append/sync, reconciliation and capture sealing.
- Host writes owner/config/decision/final native records through the bound descriptor, obtains SyncedNativePrefix, validates every occurrence/contribution, reconciles audit, then seals/adopts capture. `finish_turn` independently refuses task profile without this gate, including zero-todo turns.
- Add a narrow authorized bounded `plan_turn_entries` read for original pending AND audited entries, exact Task/attachment/turn and ordinal order. Host validates the complete native/DB occurrence set before audit/sealing, including zero-todo and already-audited siblings; no fake permit or commit replay for reads. Extend native preparation with read-only start fingerprint access and add verification-only recovery sync from expected identity/end/digest.
- V4 recovery regenerates canonical capture from authenticated expected native bytes. A seal without hash can be completed only after exact existing-seal comparison; publish the matching missing hash durably. Hash without seal or conflicting bytes refuses. Historical sealed captures retain existing prefix semantics; an unsealed latest turn requires exact file end.
- Extend the existing host retry route for task-profile lease-bound audit recovery before a seal: verify already complete native records, sync, reconcile, then seal/admit if eligible. Missing/ambiguous results remain pending. Recovery cannot append an echo, rerun provider/tools or mint an execution capability.

- [x] Test interrupted create across Task/attachment/marker boundaries: no arbitrary adoption or duplicate Task. Expose explicit owner cleanup/retry with exact frozen startup identity.
- [x] Test pre-provider restoration and authority refusals; never carry a stale preflight snapshot through setup.
- [x] Validate task native audit before excluding todo from generic observations. Omit structured input/output/digest/feedback, retain absolute ordinary source pointers, all-call limit/name bookkeeping and exact harness-note validation. Older profile planning metadata still refuses.
- [x] Test wrong/missing/duplicated/noncanonical/cross-occurrence metadata, clipped output, early loop exit, sync failure, complete-native/pre-audit crash, audit-complete/pre-seal crash and capture retry without execution.
- [x] Test V1/V2/WebV3 bytes, default marker protection, independent owner source roles, taint and strict malformed-history compatibility.
- [x] Independent host review; CLI slice must be executable before this task is complete.

### Task 5: End-to-end qualification and publication

**Files:** authored workflow under `prototypes/memory/`, required CI invocation only when stable, `docs/{HANDOFF,HISTORY,ARCHITECTURE,README}.md`, CHANGELOG, existing user memory documentation and the existing private Site.

- [x] Run actual CLI with isolated home/store, scripted local provider and graph fail-on-call sentinel: create owner task, two todo writes in a batch, change chat goal, quit/restart, restore exact DB state, update again; inspect SQLite and native audit independently.
- [x] Execute actual CLI provider-barrier interruption after DB commit; use deterministic test-only core-host injection for precise sync/audit/seal boundaries, unless an existing safe test driver provides process control. Do not ship production fault-injection environment switches. Ensure repair invokes neither model nor tools; assert no graph writes and no transcript-as-plan fallback. Report CLI and core injection evidence separately.
- [x] Run appropriate frozen workspace/default/memory builds and tests, strict Clippy/fmt; report skips and platform limits separately.
- [ ] Complete required privacy checks and publication. Existing full baseline passed 6,508 default and 7,379 memory-enabled tests before this increment.
- [ ] Broad review of this increment against its spec, followed by scoped fix review. Root updates existing draft PR and consolidated Site status; follow final CI and major/medium review loop. Do not merge or activate production.
- [x] Record implemented CLI behavior, explicit limits and next owned-web adapter; do not label web/board consumers migrated until their actual workflows pass.
