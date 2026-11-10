# Ordinary Run Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add explicit encrypted, provenance-linked episodic memory to fresh ordinary `mecha run` sessions, then recall it across processes without weakening the synthetic adapter or ordinary execution guards.

**Architecture:** Use one new authenticated observation protocol inside the existing v8 host-capture envelope and shared transaction. A core lifecycle owns host paths, source seals, recall-delivery reconciliation and capture-only recovery; thin CLI adapters attach it to the existing ordinary Agent loop. Semantic task success stays NotEvaluated.

**Tech Stack:** Existing Rust/Tokio/serde, SQLite/SQLCipher and CLI; no new dependency, schema migration, provider loop or global Config field.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.22.

## Global Constraints

- Default off behind `memory-prototype`; existing opt-out and synthetic/native v6–v8 behavior preserved.
- Protocol `ordinary-session-observation-v1`; evaluator `ordinary-session-recorder-v1`; semantic result `not_evaluated`.
- Shared v8 storage, host_capture_receipt, projection and full dependency/forgetting checks; no new DB schema.
- No model-provided paths, keys, scopes, criterion/checker or source authority. Require encrypted StorageAccess for ordinary sessions.
- Fresh SessionKind::Run only; no resume, images, no-session, synthetic overrides, program/code, late jobs, compaction/rewrite, delegation, MCP or grants. Support explicit direct tools fs_read, fs_write, fs_list, goal_context only; preserve original approval/policy, cancellation and budgets.
- No silent profile narrowing. Reject unsupported effective profiles before prompt dispatch. Reject project config rather than ignoring its restrictions. Owner-selected provider only; no fallback or hooks in first slice.
- Core owns logic; CLI parses and connects. Fixed local model service untouched; no live-data import or production activation.
- No model claims become observed artifacts or verified success. Actual tool-return status is ReportedOk/ReportedError, not task success.
- All consulted memory dependencies apply to every separately readable retained passage. Unknown/partial delivery or unsupported trace => explicit unavailable receipt.
- Only capture admission may retry; never repeat actions. Missing/edited/withdrawn source must fail closed on read and on retry.
- Retain all first failures and raw qualification evidence. Workers produce reviewed uncommitted patches; strict Spark privacy scan precedes root commit/push. Root owns shared docs and publication.

## Review Focus

- A project file/provider fallback/hook or standing grant can reach host memory outside the advertised direct tool list: Task 3 must refuse and verify zero inference on invalid profiles.
- A successful raw recall may be clipped or lost before recording: Task 2 must reconcile exact recorded results with host tracking; missing/partial is unavailable.
- Tool success and persuasive final prose cannot imply semantic success: Tasks 1/3 pin NotEvaluated even with false assistant completion prose.
- Withdrawal/forgetting between read and admission must invalidate every descendant and original passage: Tasks 1/2/3 exercise this.
- Capture failure after a successful action must preserve the action and expose failure; retry cannot execute again: Tasks 2/3 pin durable identity/idempotency.

---

### Task 1: Authenticated ordinary observation admission in v8

**Files:** Create `mecha-core/src/memory/ordinary_observation.rs`, `ordinary_observation_tests.rs`; modify `memory/mod.rs`, `memory/host_capture.rs` only as needed for shared admission and canonical routing. Existing rich/synthetic validators remain unchanged.

**Interfaces:**
- Consumes existing `HostCapture`, `CaptureReceipt`, `ExternalSourceGuard`, `Access`.
- Produces crate-private opaque `OrdinaryObservationCapture` with `try_from_host(HostCapture) -> Result<Self>`; public `SemanticAssessment` with `NotEvaluated` and `ArtifactCheck(ArtifactCheck)` as needed; constants `ORDINARY_OBSERVATION_PROTOCOL` and `ORDINARY_RECORDER_VERSION`.
- Produces `pub(crate) Memory::admit_ordinary_observation(&mut self, OrdinaryObservationCapture, &Access) -> Result<CaptureReceipt>`.
- Produces authenticated semantic accessor or helper used by new lifecycle/reader to distinguish ordinary NotEvaluated from legacy Unavailable. Existing native reader bytes must remain unchanged for existing sources.

- [x] Write failing tests: ordinary v8 capture succeeds with Unavailable/None and no artifact observation; explicit semantic assessment is NotEvaluated. Test changes to Passed, artifact bytes, duplicate/missing invocation receipts and protocol spoofing fail. Existing rich fixture must retain old bytes/verdict.
- [x] Run focused RED before implementation, preserving output (missing API compile failure is acceptable only for new symbol; add behavioral failure where feasible).
- [x] Implement separate ordinary validator and opaque adapter. Keep existing admission transaction shared: dispatch admission policy only at its front. No copy of the transaction. Reuse action validation helper if needed without changing exact-gold rich semantics. Ordinary originals are source-linked UserRequest, AssistantText, ActionInvocation and ToolResult, never ArtifactObservation. Enforce unique IDs and exactly matching invocation/receipt names; receipt status only Succeeded/Failed/Unavailable as recorded, never guessed Refused by string prefixes.
- [x] At canonical projection reconstruction, authenticate payload first, then validate the ordinary protocol specifically. Do not close historical arbitrary version labels or reinterpret all Unavailable artifacts.
- [x] Add coexistence/reopen/forget tests: ordinary and exact-gold in one v8 store; full consulted closure and source withdrawal suppress separately readable originals; retry idempotent; ordinary originals remain usable in existing independently invoked concept/interpretation paths with source checks. Retain maximum bounds already imposed by HostCapture (16 passages, 32 receipts, 64 consulted on v8, 32768 serialized bytes). Ordinary completed no-tool runs allow zero receipts exactly when there are zero invocations; legacy/rich retain their existing minimum. Reserve either ordinary protocol/recorder label against legacy/rich admission spoofing; ordinary admission rejects overflow rather than pretending complete evidence.
- [x] GREEN focused ordinary and covering host-capture/context/native-concept tests; fmt and relevant strict Clippy once at final integration. Deliver exact patch/postimage hashes and report to controller; no commit or push.

### Task 2: Core ordinary-session lifecycle, authority and recall tracking

**Files:** Create `mecha-core/src/memory_session.rs` with focused child modules `memory_session/{authority,recording,recall,tests}.rs` as needed; declare feature-gated module in `lib.rs`. Minimal crate-private reusable helpers in existing bridge/GoalMemory only when shared rather than copied. Do not modify CLI or synthetic protocol behavior.

**Interfaces:**
- Consumes Task 1 opaque admission and shared v8 Memory/GoalMemory APIs.
- Produces serde host DTO `SessionMemoryRequest { protocol, db: PathBuf, key_file: PathBuf, authority: PathBuf, domain, account, scopes }`, deny unknown fields, protocol `ordinary-session-memory-v1`, max request 65536 bytes. Paths are absolute and host-owned, not tool input. Expose a core request-path admission helper so CLI can also exclude the manifest itself from the task workspace and aliases.
- Produces `SessionMemory::load_request(path)`, `SessionMemory::initialize(&request)`, `SessionMemory::open(&request, workspace)`, `open_for_recovery(&request)` (recovery handles cannot begin or create tools), `begin(&self, session: &Session, prompt: &str, goal: &GoalRef) -> Result<SessionMemoryRun>`, `goal_memory(&self, &SessionMemoryRun) -> Result<Arc<dyn Tool>>`, `finish(&self, &SessionMemoryRun) -> SessionMemoryReport`, `retry(&self, run_id: &str) -> SessionMemoryReport`, `forget(&self, run_id: &str) -> Result<ForgetReport>` (adjust lifetimes/Arc mutability internally without changing responsibilities; signal needed signature changes before coding).
- `SessionMemoryReport` serializable host-only protocol/run_id/status (`retained`, `unavailable`, `pending`, `forgotten`), semantic_assessment `not_evaluated`, optional capture receipt and fixed reason enum; no private path/key/raw diagnostics. Opaque run handle exposes run_id. Public API never executes a provider or action.
- Produce core admission helper taking effective tool names, workspace and actual grant state; CLI supplies config restrictions separately. Only allow fs_read, fs_write, fs_list, goal_context, with goal_context required.

- [x] RED: a real SessionKind::Run transcript with goal, config, owner message, fs_write invocation/result, final assistant, taint and outcome is retained; reopen guarded recall yields it; fake assistant success remains unverified. Existing Test sessions still refused by ordinary lifecycle.
- [x] Initialize NEW v8 encrypted DB and NEW private authority explicitly (no overwrite or plaintext fallback); open requires existing compatible DB/authority. Key outside workspace, DB parent and authority; paths/aliases excluded from workspace and grants. Private authority uses no-follow bounded regular-file reads and private create/fsync/no-clobber seals. Reuse existing private-key policy and storage APIs. Source binding binds authority identity, session ID, exact prefix bytes, request/goal, final taint and authenticated recall tracking. Missing/edit/rewrite/withdrawal makes it unavailable; later append must not silently authorize a different run.
- [x] Begin before provider execution seals one fresh session admission and frozen goal/prompt (prompt <=2048 bytes, same owner prompt used as intended criterion; no invented task gold). Finish strictly parses the original JSONL (max4MiB): require exactly one Run header/config, confirmed same goal, bounded messages, final Taint and Outcome; accept ordinary empty Grants, Summary and host Notes as audit-only. Assistant Thinking and opaque signatures remain only in the original hash-bound JSONL; skip import without changing later block indices. Wrong-role Thinking and thinking-only final messages refuse. Unsupported records/blocks/compaction/code/late jobs/malformed/torn input withhold. Do not use lenient Session::parse as authority. Host extensions are audit-only only when proven known generated notes; ambiguous Extend refuses. Bind all actual input/result locators and digests, preserve attribution, reject overflow rather than silent truncation. No ordinary ArtifactObservation in this slice.
- [x] Wrap GoalMemory only for host delivery tracking. Pending attempt before call; record raw successful packet and full canonical dependency set. Match tracked results against unique exact recorded goal_context call/result envelopes, including input digest and wrapper handling. Do not copy serialized recalled packets into new original passages. Empty success differs from failure. Refused/failed reads may admit only when authenticated as zero delivery; otherwise unavailable. Any unaccounted/partial delivery withholds all capture. Cap tracking (32 attempts, existing20KB packet each), never reset tool budgets.
- [x] Seal finalized payload BEFORE SQLite admission; retry loads immutable sealed input and reauthorizes everything, never reruns tools. Failed/incomplete/provider-error paths return explicit unavailable receipt; no fabricated Outcome. Exact duplicate finalization returns original receipt. A pending admission after successful action remains retryable; changing source or seal refuses. fsync at the new boundary before claiming durable seal.
- [x] Forget by run_id uses existing Memory::forget on authenticated source key, purges complete episode/descendants and records monotonic external withdrawal; no automatic search of other stores. A source JSONL removal immediately withholds; forgetting explicitly purges selected DB but does not promise deletion of external backups/files.
- [x] GREEN lifecycle tests: source withdrawal during recall/commit, wrong domain/scope/key/path aliases, source edited after seal, replay/duplicate attempts, crash after seal/after DB commit, failed/empty/missing/clipped recall, cancellation/provider error, unsupported trace, failed memory never destroys recorded action. Test source hooks with deterministic fixtures rather than rereading private stores.
- [x] Deliver core patch and report, API manifest and test evidence for Task 3; no commit/push or service changes.

### Task 3: Ordinary CLI opt-in and two-process qualification

**Files:** Modify `mecha-cli/src/commands/run.rs`, `main.rs`, `commands/mod.rs`; create thin `commands/memory_session.rs`; create `mecha-cli/tests/ordinary_memory_cli.rs`; feature gates follow `memory-prototype`. Root owns docs/CHANGELOG/Site.

**Interfaces:**
- Consumes Task 2 request/lifecycle/report.
- `mecha run --episodic-memory <request.json> --goal KIND:ID ...` fresh-only.
- `mecha memory-session --request <request.json> init|retry <run_id>|forget <run_id>` are host-only no-model operations, JSON reports; no synthetic acknowledgement or global discovery. No new Config field.

- [x] RED clap/API test: opt-in flag and host-only init/retry/forget commands absent on baseline; opt-out JSON unchanged.
- [x] Add opt-in conflicts with resume/no_session/images/mismatch_case/memory_context; require --goal, --no-mcp, --no-code-tool, --no-grants and explicit --tool selection within core allowlist. Reject project `cwd/mecha.toml` presence (including symlink/unknown metadata) rather than discarding project approval restrictions; use existing owner-only loading for opt-in before router hold/config observation. Reject valid MECHA_SESSION_KIND Test/Experiment/Driven overrides and driven marker. Require effective no hooks/fallback and admitted final tool list/empty grants; no implicit weakening/narrowing of security. Ordinary provider/permission/budgets/taint remain unchanged. Invalid request/profile must fail before prompt dispatch; validate paths/request before startup router network where feasible.
- [x] After ordinary setup and before RunConfig recording, attach wrapper over admitted goal_context. Begin after fresh session/confirmed goal and before user run. Keep existing single interruptible Agent run. After record_run/taint/outcome, finish before JSON printing and process::exit. Add optional `memory` report only on opt-in. Failed capture does not discard the answer/outcome; prose stderr reports a fixed status/run ID. Fatal provider error should persist unavailable/pending capture status without inventing an Outcome, then return original error. Preserve session hooks and native exit behavior. Recovery commands must never load provider/setup/router.
- [x] Actual CLI two-process HTTP fixture in isolated temp HOME/MECHA_HOME config: process A writes an artifact and falsely claims unrelated success; process B has different workspace/goal, no answer in prompt, obtains prior observation in actual provider wire via goal_context, writes distinct result. DB retains NotEvaluated, attribution and consulted closure. Use explicit loopback compatible provider/SSE support, not real user config; remove corpus override only inside isolated test environment. Keep raw wire/output/report for qualification.
- [x] Cover approval denial/read-only mode and cancellation/budget behavior, no opt-in unchanged JSON, existing synthetic path unchanged, malformed/unsafe manifests, project provider/hook/fallback guard, no plaintext fallback, unsupported tools/aliases/grants, capture-only retry after forced admission failure without another provider call/action, forget/withdrawal between delivery and finalization prevents dependent retention. Tests must inspect artifact/transcript/DB/wire, not merely exit code.
- [x] GREEN focused CLI tests and root covering/core suites. Deliver patch/report to root. Do not run real-model experiments, commit, push or deploy; root owns final source qualification and publication.

### Task 4: Combined qualification and publication (controller)

- [x] Fresh combined code review against baseline165e7cd; one scoped fix wave if needed. Preserve rulings and minor followups.
- [x] Run required fmt, all-target/all-feature strict Clippy and workspace tests; default and memory builds, standalone and Python compatibility; Linux required-backend/MSRV on isolated Spark checkout. Use previous qualification helpers with new namespace, no service changes. No test during source mutation.
- [x] Run authored two-process ordinary CLI gate. If model access is available, run one declared isolated two-run workflow using resident model and no owner data; report mechanics separately from decision/learning benefit. Preserve first failures, never silently replay actions.
- [x] Run handoff skill for affected memory scope; update design/HISTORY/HANDOFF/CHANGELOG/user commands. Consolidate Site HOME capability/remaining gates and add detailed evidence to archive; preserve previous failures and audience.
- [ ] Strict staged-source and commit-message privacy scans on Spark with required owner corpus, normal hooks, commit/push existing draft PR846. Follow exact-head CI/review to terminal; no merge or production activation. Preserve evidence/worktrees per user requirements.

Publication status and exact-head CI/review are tracked on draft PR #846; the final publication gate above is external to this source snapshot. Qualification and first failures are recorded in HISTORY.
