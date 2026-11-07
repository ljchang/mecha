# Appraisal bridge implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. The owner explicitly authorized implementation and subagents; no further design approval is pending.

**Goal:** Connect ordinary guarded task execution, independently checked outcome, existing deterministic appraisal and source-linked memory, then recall it in another process under a changed constraint.

**Architecture:** A default-off core adapter captures a safe projection into an explicitly created v6 store. Host-only appraisal/sensors remain outside reader SQLite. An injected source authority guards supported reads; unsupported legacy producers fail closed for these records. Existing executor, planning feedback and appraisal semantics remain authoritative.

**Tech stack:** Existing Rust core/CLI, SQLite, serde, authored Provider integration fixtures. No new dependency or production migration.

**Spec:** docs/MECHA-MEMORY-DESIGN.md §31; exact implementation contract below.

## Global constraints

- Synthetic-only explicit paths/account/domain/scopes, no implicit owner stores or production configuration discovery.
- Original JSONL remains authoritative; read once for digest, parsing, taint and appraisal input.
- No transaction spans execution or inference. Capture retries never execute actions.
- Source current restriction/withdrawal/forget wins over historical snapshots; unknown authority withholds.
- No sensor, threshold, numeric valence, full Appraisal/RunStats or diagnostic rationale enters model-readable SQLite/FTS/tool output/errors.
- Existing Clean-only interpretation policy and private/untrusted taint remain unchanged.
- No second executor, nightly migration, owner knowledge merger, new model call or automatic concept promotion.
- Root owns shared docs, PR and Site. Workers own disjoint files below; coordinate shared signatures before changing them.

## Review focus

- Every serving door and transitive dependency must enforce external authority, including ranking statistics and final returned bundles.
- Root capture alone cannot establish provenance; receipt/dependencies/guard bindings and forgetting must be atomic.
- Missing or ambiguous event-time goal/check binding must not acquire the last session goal.
- An authentic successful tool result is not an independently successful task; missing checks remain unknown.
- v6 must neither downgrade nor enter older durable tables absent from its schema.

### Task 1: Store and guard-aware read paths

**Files:** memory/host_capture.rs and tests; memory/mod.rs, store.rs, sql.rs, recall.rs, context.rs, forget.rs; affected durable version guards; tool/goal_memory.rs and tests.
**Interfaces:** Implement the exact shared contract below. Runtime worker owns lib.rs and consumes HostCapture/guard interfaces.

- [x] Add failing tests for atomic typed admission, exact repeat/conflict, real dependencies, external missing/unavailable/revoked authority, unsupported consumers and v6 downgrade refusal.
- [x] Implement v6 new-file schema, safe record rendering, typed crate-only admission, receipt/tombstone semantics and guard-aware authorization.
- [x] Cover read/search/recall/mention/context/ranking/bundle/read-after-reopen; final returned closures recheck authority. Default unsupported consumers withhold.
- [x] Verify forget of original/projection erases content/FTS/bindings and suppresses retry/new-prefix reuse; old schema behavior preserved.
- [x] Run focused tests, self-review and report exact red/green evidence. Independent task review before integration acceptance.

### Task 2: Host capture adapter and existing appraisal

**Files:** memory_experience_bridge.rs and tests; appraisal_store.rs narrow shared from-text seam. Runtime worker owns lib.rs declaration after agreement; no store-owned files.
**Interfaces:** Consume HostCapture, CaptureReceipt, ExternalSourceGuard from Task1. Runtime API must accept explicit host source/check/goal/joined-store inputs; no model-constructible execution credential.

- [x] Add failing tests for same-byte transcript/appraisal/provenance, event-time single confirmed goal, native call/result linkage and independent artifact checks.
- [x] Implement bounded local file authority and opaque audit binding, typed sensor-free projections and host-only appraisal audit. Use Session::parse/from-text evidence from one read; no ambient Stores::load or Homeostat survey.
- [x] Implement capture-only retry after recorded execution, strict immutable source/audit verification and withdrawal status. Never re-execute from capture.
- [x] Qualify tool-success/wrong-artifact, missing sensor, malformed/changed goal, source mutation, tainted source and safe errors; scan complete stored closure for host canaries.
- [x] Run focused tests, self-review and report. Independent task review before integration acceptance.

### Task 3: Connected cross-process qualification and delivery

**Files:** mecha-core/tests/memory_bridge.rs; core example or thin CLI route agreed with runtime API; prototype usage and existing docs (root). Qualification worker owns integration test/example; runtime worker owns its module declarations.
**Interfaces:** Real Agent::run_in with ordinary FsRead/FsWrite/TodoTool, authored provider, native Session recording, Task2 capture, Task1 GoalMemory::open_with_source_guard.

- [x] Run one synthetic CSV-to-JSON task with actual tool dispatch, plan and independent host artifact check; retain authentic transcript and explicit host goal/sensor context.
- [x] Reopen DB in a separate OS process, construct actual GoalMemory with host authority, execute second ordinary task with changed criterion and independently check its artifact. Inspect actual provider requests and private/untrusted taint.
- [x] Exercise successful process/wrong artifact, source withdrawal/forget, missing authority, changed goal, host sensor canaries and crash-after-execution capture-only retry with unchanged dispatch count.
- [x] Run workspace build/fmt/strict clippy/tests and feature-enabled focused/full qualification on Spark as appropriate. Authored-provider success is not a real-model benefit claim.
- [x] Independent integration/whole-change review; resolve major/medium findings.
- [x] Update owner-private Site with exact implemented scope/limits.
- [x] Prepare draft PR update and required owner-corpus privacy gate. Publication completion is recorded by the PR head/checks; do not merge or deploy production.

## Follow-up gate after automated review

- [x] Refuse native code/program captures and prior bindings, before evidence assembly; scoped rereview and focused tests pass at `12e2432c4`.
- [x] Finish exact-fix full Spark build/fmt/Clippy and serial default/feature suites.
- [x] Finish retained demonstration with both independent artifact checks.

Publication gate: scan the final commit range against the owner corpus before
push; the PR head/checks record publication and the final automated review.

## Integration decisions

The supported completed range rejects native late results/failures and resumed
messages without a matching outcome. After capture, only complete cost-summary
records may extend the pinned source; semantic appends withhold that binding.
The synchronous exact-artifact checker uses Passed/Failed/Unavailable/Refused;
Pending remains a type for later adapters, not an invented asynchronous state.

Native memory results must supply complete source lineage. The adapter derives
or validates their returned references before passage truncation; separately
readable originals inherit consulted dependencies as well as the projection.
This conservative capture-wide inheritance may withhold unrelated passages when
one consulted source is withdrawn. Existing revision lineage is immutable;
unsupported dependency-changing source reuse refuses rather than mutating it.
Legacy record decoding and unguarded experience context semantics remain intact.

## Shared interface contract

# Appraisal bridge: frozen A+B implementation contract

Source-reviewed against HEAD 8faee592, 2026-11-07. Implements §31 A+B with authored providers; no benefit, production, or interpretation-producer claim. No unresolved owner decision blocks implementation.

## Ownership

- Store worker: `memory/host_capture.rs`, its tests, `memory/{mod,store,sql,recall,context,forget}.rs`, `tool/goal_memory.rs` and associated tests. Own all shared module declarations for these files.
- Bridge worker: `memory_experience_bridge.rs`, its tests, and the narrow from-text seam in `appraisal_store.rs`. Bridge worker owns its `lib.rs` declaration; qualification worker owns the example/support fixture and subprocess tests. Bridge worker consumes the signatures below; do not edit store-owned files.
- Root owns docs/changelog/handoff and integration. Coordinate any signature change before editing consumers.

## Guard and supported read paths

In `memory::host_capture`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalBinding {
    pub authority: String, // fixed host adapter/version identifier, <=64 ASCII
    pub locator: String,   // opaque sha256 hex, never filesystem path or JSON
    pub revision: String,  // sha256 hex of authoritative inputs, including status binding
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus { Checked, Withheld, Unavailable }
pub trait ExternalSourceGuard: Send + Sync {
    fn check(&self, binding: &ExternalBinding) -> SourceStatus;
}
```

Synchronous bounded local checks only: no network, no model, no writes. Guard implementation belongs to host bridge, injected as `Arc<dyn ExternalSourceGuard>`. It resolves locator using host-owned configuration, verifies exact prefix/check/goal audit revision and current withdrawal state. Missing authority, unknown locator, missing source, truncated/revised prefix, unavailable state and withdrawn source all withhold. Errors never include source bytes, paths, appraisal values or host diagnostics in tool answers. Appending after the pinned prefix is permitted only if the authority continues to affirm that exact historic binding; new prefix is a different assessment version.

Memory keeps `Option<Arc<dyn ExternalSourceGuard>>` initialized to None by existing constructors. Add:

```rust
impl Memory {
    pub fn create_host_capture(path: &Path, domain: &str) -> Result<Self>;
    pub fn with_source_guard(self, guard: Arc<dyn ExternalSourceGuard>) -> Self;
}
impl GoalMemory {
    pub fn open_with_source_guard(
        path: &Path, access: Access, account: String, situation: Situation,
        guard: Arc<dyn ExternalSourceGuard>,
    ) -> Result<Self>;
}
```

The existing no-guard constructors still open files; unguarded legacy records remain eligible. Missing guard withholds guarded records, not the whole store. No positive guard verdict is cached across calls.

Keep `sql::authorized(conn, reference, access)` as the conservative default: it now delegates to `authorized_with_guard(..., None)`. Add the latter with a final `Option<&dyn ExternalSourceGuard>` parameter. In the existing closure traversal check every external binding as well as current restriction and scope. Tables absent on v1–v5 mean no external bindings; malformed bindings fail closed. Do not change freshness semantics.

Thread the optional guard through the supported serving paths: `Memory::read`, `search`, `recall`, `recall_mention`, `context`, including eligible-index construction, neighbors, source selections, and complete knowledge bundles. Each dependency independently authorizes. Default-authorized callers in concept preparation/retention, plan assessment, decisions, experiences and durable lifecycle APIs deliberately refuse guarded records in this increment. They may not use raw `lookup` to return those records without authorization. Audit public status/spec APIs too: unsupported v6 lifecycle operations must refuse explicitly rather than return unchecked payloads.

Recheck external bindings for final returned closures immediately before the read method returns (within its existing SQLite transaction where practical). This is point-in-time authorization: revocation after the final check and before receipt by a caller is outside the guarantee. There is no claimed atomic filesystem/SQLite snapshot. Transactions never span dispatch or inference. Source failure before final check withholds the entire affected bundle.

## v6 admission and receipt

`create_host_capture` creates a NEW regular prototype file with existing 0600/create_new semantics and base schema, plus bridge tables and user_version=6, atomically before returning. Existing create remains v1. v6 deliberately contains no durable lifecycle tables: update the version-based `durable*::extend_forgotten` and `invalidate_forgotten` hooks to skip v6, and refuse unsupported v6 lifecycle entry points. Store worker owns these narrow durable-module changes. Open/read-only accept 1..=6; prechange binaries reject 6. No automatic migration of v1–v5. Prevent legacy schema initializers from downgrading a v6 file; unsupported lifecycle writes fail explicitly.

Tables (column spellings internal, semantics fixed):

- host_capture_receipt: identity hash primary key; payload hash; receipt id; projection object/revision; serialized ordered original references; state active/forgotten. Identity tombstones survive forgetting, but safe payloads, references and source bindings of forgotten records do not.
- external_binding: revision primary key referencing revision; binding fields. Every imported original AND projection has a binding. A projection also has real dependency edges to all its originals and consulted revisions. A descendant is guarded through closure even without its own binding.

All identifier/digest strings canonical and bounded. Identity is domain/account/session/prefix_digest/check_id/check_version/evaluator_version. The identity includes frozen criterion revision through check_id (digest of criterion plus checker definition). Payload hash covers complete canonical typed input, including source passages, dependency order, scopes, occurrence times and binding; no arbitrary JSON.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureIdentity {
    pub domain: String, pub account: String, pub session_id: String,
    pub prefix_digest: String, pub check_id: String,
    pub check_version: String, pub evaluator_version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureReceipt {
    pub id: String,
    pub projection: RevisionRef,
    pub originals: Vec<RevisionRef>,
}
```

Admission below is crate-visible only and never registered as a model tool. Validate domain against both store and Access; account against each source. Acquire one immediate transaction, validate suppression, current/fresh/authorized dependencies and guard, then write originals, projection, dependency edges, bindings and receipt. Any failure rolls back every row. All dependency scopes union into output scopes. Existing consulted dependencies must be sensor-safe bridge originals/projections with complete verified safe closure; refuse arbitrary legacy Evidence roots because they can contain raw host records.

Exact retry with still-valid dependencies/authority returns identical receipt. Conflicting payload at the same identity refuses, including changed scopes. Check validity BEFORE returning an existing receipt. Forgetting a source purges its whole dependent closure and marks affected receipts forgotten; their opaque identity hash tombstone blocks late capture. A new prefix cannot bypass the source suppression digest. Choose stable SourceKey IDs per session/source field, not a fresh UUID on retry. Forgetting either original or projection must invalidate its receipt.

## Allowlisted DTO

Use these types in `memory::host_capture`; derive serde with deny_unknown_fields for structs. They are trusted-host input, not a generic JSON deserializer exposed to models.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassageKind { UserRequest, AssistantText, ToolResult }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OriginalPassage {
    pub source: SourceKey,
    pub kind: PassageKind,
    pub line: u64,             // physical authoritative JSONL line, 1-based
    pub pointer: String,       // known message string field only
    pub text: String,          // exact original contiguous passage, <=4096 bytes
    pub occurred_at: Option<String>, // exact event time or None, never session start
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptStatus { Succeeded, Failed, Refused, Pending, Unavailable }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolReceipt {
    pub call_id: String, pub tool_name: String,
    pub status: ReceiptStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckVerdict { Passed, Failed, Refused, Pending, Unavailable }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactCheck {
    pub artifact_digest: Option<String>, pub verdict: CheckVerdict,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfirmedCriterion {
    pub goal: String,          // parses as GoalRef
    pub goal_text: String,     // <=2048 bytes, ordinary owner text
    pub criterion_text: String,// <=2048 bytes, ordinary acceptance condition
    pub criterion_revision: String, // sha256
    pub confirmation_digest: String,// opaque host confirmation provenance
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostCapture {
    pub identity: CaptureIdentity,
    pub binding: ExternalBinding,
    pub criterion: ConfirmedCriterion,
    pub originals: Vec<OriginalPassage>, // 1..=16
    pub receipts: Vec<ToolReceipt>,      // 1..=32
    pub artifact: ArtifactCheck,
    pub consulted: Vec<RevisionRef>,     // <=32, safe bridge closure only
    pub scopes: BTreeSet<String>,
    pub occurred_at: Option<String>,
}
impl Memory {
    pub(crate) fn admit_host_capture(
        &mut self, input: HostCapture, access: &Access,
    ) -> Result<CaptureReceipt>;
}
```

Identifier fields max 256 bytes except fixed sha256/authority. Pointer max 256 bytes, total serialized allowed projection <=32768 bytes. No free-form diagnostic, rationale, assessment score, arbitrary metadata or tool argument field. Fixed rendering of these typed facts becomes ordinary `Record::Evidence(Capture)` for retrieval; no new Record enum variant needed. Original source records contain only exact allowlisted passage text (not JSONL line bytes). Do not use `import_session_jsonl`. Persist source positions in typed sensor-safe projection text or opaque binding audit as appropriate; never put raw host rows into dependencies. `TranscriptMeta` is not appropriate for a copied whole host line.

Opaque locator/revision are safe hashes; host full audit contains exact appraisal inputs/version and original taint. Host audit is outside reader SQLite, including errors. The check verdict is independent from tool status: a successful command can produce a failed artifact. No owner acceptance is inferred.

## Host adapter and event binding

Bridge worker may expose a narrow public capture function with its own host-input struct; its only store integration is constructing HostCapture and calling admit_host_capture. Inputs must explicitly name synthetic transcript path, bounded artifact path/checker, frozen criterion, explicit joined SessionRecords and explicit host audit/authority paths. No default owner loader. The adapter reads authoritative bytes once, parses the same bytes using a shared SessionEvidence from-text seam, resolves appraisal::for_transcript with explicit records, validates native tool-use/result pairs and independently reads/checks artifact bytes. Do not accept a caller `executed=true`, model-authored check result or JSON outcome as proof. Snapshot/digest/audit is completed before calling admission.

Existing `SessionEvidence::read_with_transcript(path)` reads internally. Add `pub(crate) fn from_transcript_text(path: &Path, text: &str, ended_at: Option<DateTime<Utc>>) -> Result<(Transcript, SessionEvidence)>` in appraisal_store.rs, factoring its existing Session::parse/messages_ever/programs/of logic unchanged; existing read_with_transcript calls it after its read/mtime capture. Bridge calls it with its one captured byte buffer. This avoids inconsistent digest/provenance snapshots and preserves existing unknown-taint behavior.

First increment requires one host-confirmed criterion for the entire captured prefix. Confirmation is established by trusted host setup before dispatch and checked against recorded configurations/anchor timeline, not inferred from final GoalRef. Refuse changed/ambiguous/missing event-time binding, summarizing rewrites, multiple criterion versions, and unsupported resumed ranges. Same-prefix persistence retries work; new prefix is a separately versioned session assessment, never an additional independent success count. Capture is not a scoring accumulator.

For B use a separate ordinary session with a different explicitly confirmed criterion; reopening old memory does not rewrite its goal. New GoalMemory binds the new situation, retrieves historical observation as external evidence, and actual executor writes a separately gold-checked artifact. Historical assessment remains host-only; new task interpretation is the second run's ordinary planning, not a synthesized trusted rule. No taint clearing or Clean bypass.

## Required focused acceptance

1. Typed atomic admission and exact retry; conflict refuses; stale/inaccessible/forgotten dependency causes zero rows; arbitrary sensor-bearing consulted root refused.
2. No guard/unknown authority/guard unavailable/source edit/truncate/delete/withdraw withhold read, search, recall, mention, neighbors, context source selection, dependency bundles and reopened GoalMemory. Unguarded legacy fixtures still work. Alternate concept/durable APIs refuse bridge refs; v6 cannot downgrade.
3. Forget original and projection cases remove FTS/dependencies/binding content, tombstone receipt identity, block retry/new-prefix reuse of suppressed source. Existing v1–v5 open semantics and bytes remain intact.
4. Ordinary dispatcher CSV→JSON task with actual plan/check records; process-success/wrong-artifact yields Succeeded + Failed; absent/refused/pending independent check remains distinct. Missing sensor never becomes zero or task failure.
5. Change goal within session refuses capture, while second-session changed goal retrieves prior factual evidence without reattribution. Malicious tool passage stays external and ordinary private+untrusted chosen-egress interlock fires.
6. Crash after actual dispatch and before capture: reopen transcript, retry capture only; same receipt and unchanged dispatch count. Never rerun uncertain effects.
7. Host canaries in sensors, thresholds, full Appraisal, RunStats, Homeostat and rationales absent from every SQLite table/FTS/dependency payload, tool response and errors. Host audit may contain them outside reader DB. Audit safe closure, not only projection rendering.
8. Separate OS process constructs GoalMemory with freshly injected authority, retrieves prior observation, executes changed-criterion run and independently grades second artifact. Record no positive benefit claim from authored-provider success.
