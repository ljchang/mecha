# Native-session vector recall implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Root owns publication; workers freeze reviewed patches, not commits.

**Goal:** Connect checked vector recall to native sessions and measure independently checked decision outcomes against matched lexical recall.

**Architecture:** Add a run-local host control to GoalMemory and an additive native-session sibling. Reuse existing packet assembly, execution, artifact checking, appraisal and recapture; use a bounded experiment driver rather than a new agent executor.

**Tech Stack:** Existing Rust/Tokio/SQLite/checked Embedder, Python standard-library experiment orchestration. No new dependencies or schema.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.19.

## Global constraints

- Baseline `9dc6f4368615d6964f25c9952e46f8ec554f9123`, existing isolated branch; exact baseline CI and full Spark qualification already passed. Reuse that evidence without repeating unchanged suites.
- Synthetic-only, explicit host inputs, no model-discovered endpoint/profile/key; default behavior unchanged. Keep owner graph independent.
- Exact nonempty query <=4,096 bytes, one atomic attempted call; three anchors/zero neighbors/20,000 bytes; fixed 15-second whole retrieval budget. Existing native six-turn/16,384-output/180-second budget stays.
- Recheck provenance/current restrictions around embedding and capture; no transaction or mutex crosses inference. Preserve ContextPacket and taint shape.
- Lexical/dense pair with frozen8cases (4paraphrase/2exact/2no-evidence), isolated complete seed DB+authority copies and immutable shared originals; first reports retained.
- All product changes reviewed before model scoring. Root runs strict Spark staged/message scans before normal-hook commit/push. Keep PR draft; no merge or production activation.

## Review focus

- Parallel/malformed tool calls must not obtain additional evidence: atomic claim and rejection tests in Task1.
- Cancellation by dropping a future must not report NotCalled or Completed: drop-safe monitor and no late output tests in Task1/2.
- Current restrictions can change without knowledge sequence: guarded fresh finalization and capture-withdrawal tests in Task1/2.
- A correct artifact can coexist with absent/fallback/errored recall or invalid lineage: separate protocol/outcome checks in Task2/3.
- Controlled tool descriptions must agree with the one-call schema; preserve legacy descriptions. Audit pre-dispatch tool attempts separately from entered GoalMemory call counts.
- Positive evidence-qualified success requires the decisive original observation in the delivered packet and capture closure; raw guessed correctness is separate.
- Historical snapshot alone cannot isolate arms; copied state, immutable source and request-leakage audits in Task3.

### Task1: Controlled GoalMemory retrieval

**Files:** modify `mecha-core/src/tool/goal_memory.rs`; add a private helper `mecha-core/src/tool/goal_memory_recall.rs` and focused tests; minimal module declarations only. Existing goal_memory tests remain covering legacy routes.

**Interfaces:** Public facade reexports `NativeRecallMode::{Lexical,Dense}`, validated private-field `NativeRecallConfig`, cloneable `NativeRecallHandle`, serializable `NativeRecallReport`. Constructors `NativeRecallConfig::lexical(query:String)->Result<Self>` and `dense(query:String, endpoint:String, profile:EncodingProfile)->Result<Self>` validate all host settings. `GoalMemory::with_native_recall(self, config:NativeRecallConfig)->Result<(Self,NativeRecallHandle)>`. `NativeRecallHandle::report(&self)->NativeRecallReport` snapshots host-only accounting. A `protocol_satisfied()` predicate requires exactly one valid completed requested-mode call and no rejections; empty eligible completed calls qualify, fallback does not. Config metadata identity must be readable without exposing private runtime internals to Task2.

- [x] Write RED tests for semantic-only evidence via actual local HTTP, same-query one-call enforcement including concurrent calls, no-memory/bindings/interpretation refusal, live external revocation, cancellation/drop, timeout and nonvacuous mutex/transaction-release check.
- [x] Run the smallest new tests against old lexical behavior; preserve actual failed assertions. Compile-only failure is not sufficient for behavior regression.
- [x] Implement bounded async prepare/embed/finalize with one atomic claim and drop-safe host reporting. Keep legacy delegation and packet/taint behavior unchanged. Test constructors reject invalid endpoint/query before any HTTP.
- [x] Verify covering GoalMemory, checked embedding and hybrid tests plus strict scoped feature Clippy. Freeze exact diff/postimages/report for independent task review; no worker commit or push.

### Task2: Native-session and thin CLI wiring

**Files:** modify `mecha-core/src/memory_situated_workflow.rs`, focused native integration tests, `mecha-cli/src/commands/memory.rs` and `mecha-cli/tests/memory_cli.rs`. Add a focused wire/helper module only if needed.

**Interfaces:** consume Task1 types. Add `run_native_session_with_recall<F,Fut>(database:&Path, storage:StorageAccess, authority:&LocalAuthority, request:&NativeSessionRequest, recall:NativeRecallConfig, root:&Path, factory:F, cancel:&CancellationToken)->Result<TaskReport>` with existing provider factory bounds. Existing functions route through shared private logic with None. TaskReport adds optional recall report omitted when absent. Add core-owned deny-unknown-fields `NativeRecallRequest` tagged lexical/dense with query and explicit dense endpoint/alias/artifact/dimensions/pooling/normalization, convertible to validated config using existing fixed encoding representation. CLI adds optional `--recall-request` absolute JSON path, max1MiB, delegates to the sibling; it does not implement retrieval logic.

- [x] Add RED native tests proving actual Agent GoalMemory retrieval influences checked artifact and is retained with original consulted closure; include missing call, rejected extra call, source withdrawal after delivery, encrypted storage and unchanged legacy wire behavior.
- [x] Wire durable started identity, run cancellation, host audit handle and optional final report. Validate incompatible interpretation/invalid recall before output root/provider discovery. Keep existing bridge parser/capture refusal unchanged.
- [x] Add actual CLI refusal/no-overwrite and authored HTTP lifecycle checks. Controlled unsatisfied protocol/run/capture errors preserve report and return nonzero; failed artifact alone remains a recorded result.
- [x] Run covering native/core/CLI tests and strict scoped Clippy. Freeze exact task delta/report and receive independent review before Task3 runs full integration.

### Task3: Frozen decision-benefit trial and publication

**Files:** tracked portable synthetic fixture/protocol and experiment-only driver under `prototypes/memory/`, driver tests under its tests directory. Root updates existing design/architecture/history/handoff/README/changelog/CLI reference and private Site.

**Interfaces:** invoke actual `memory native-session --recall-request` and existing initialization. Seed native experience using authored loopback provider responses (actual execution/capture). Freeze closed v8 DB, complete authority, original file hashes,8paired tasks and host gold before scored calls. Driver emits durable planned-slot/started/completed/failed reports with raw artifact outcomes, qualified decision success, recall disposition, original lineage and costs. Core check/recapture receipts are authority; independent audit reads original artifacts/DB/fixtures instead of trusting summary labels. Protocol bytes ship and digest is computed; runtime identities remain separate immutable receipts.

- [x] Freeze and independently review fixture/protocol, leak checks, pairing/isolation and arm order before any real inference. Case authoring remains separate from implementation outcome inspection.
- [x] Implement orchestration and meaningful authored tests for no replay/no overwrite, exact shared initial state, failures retained, no gold/source bypass, independent grading and seed/arm lineage. No new executor or production path.
- [x] Run required final fmt/build/default and feature workspace tests, standalone/Python, strict Clippy and MSRV. Preserve first failures and qualify only changed scopes again after fixes.
- [x] On Spark verify existing resident chat/embedding identities under holds, frozen source/binary and resource conditions; run one frozen16-session comparison with counterbalanced order. No service/model changes. Preserve first results and independently audit.
- [ ] Root finalizes docs from evidence, runs strict staged/message privacy scans, pushes existing draftPR846, follows exact-head CI/review, and publishes private Site status. Record limits and owner minor follow-ups.
