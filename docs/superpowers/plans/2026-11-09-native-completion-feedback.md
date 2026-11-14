# Bounded native completion feedback implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Root owns docs, integration, privacy scans and publication; workers freeze reviewed patches, not commits.

**Goal:** Give an explicit native session one bounded chance to complete missing required operations without leaking semantic gold or resetting execution authority.

**Architecture:** Add a generic optional completion observer to the existing Agent loop, then an explicit native artifact/recall adapter and thin CLI argument. Keep one loop-owned report and reuse existing execution, safe artifact reads and source capture.

**Tech Stack:** Existing Rust/Tokio/async_trait/SQLite and Python standard-library orchestration; no dependencies or schema changes.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.20. The user's authorization to continue and publish remains in effect; no additional design approval is required.

## Global constraints

- Baseline `663fbba95f22310a9035bbbe51926348f7ae2b53`; exact source qualification and CI18/18 verified in the baseline receipt. No duplicate unchanged suite run.
- Default behavior and legacy native JSON unchanged. Explicit synthetic native route only; no production, owner-graph or nightly migration.
- One appended feedback intervention inside the original six-turn/16,384-token/180-second run; each observer call at most two seconds within remaining time. Cancellation/refusal/closing/truncation/actual ceiling stops win. A normal EndTurn at a turn/token/cost cap may observe readiness within wall time but cannot append feedback or resume.
- No gold, artifact prose, dynamic failure text or model-authored instructions in feedback. Fixed generic operational vocabulary, harness role/voice, no independent evidence.
- Optional completion shape: exact 1..32 rows; 1..16 unique keys each 1..64 ASCII identifier bytes; all values strings. No derivation from expected_rows. Reuse 1MiB no-follow reader and duplicate-key refusal.
- Workers use separate managed worktrees. One local Cargo writer with2jobs; root integrates patches sequentially and owns shared docs. Full final Spark checks and strict staged/message privacy before commit/push; preserve failures. PR846 stays draft/unmerged.

## Review focus

- Late-ready observer/cancellation must not publish Ready or append feedback: Task1 deadline race and dropped-future tests.
- Valid-but-wrong JSON must receive no semantic hint: Task2 explicit wrong-value/unknown and unrelated hidden-gold tests.
- Already consumed recall must not acquire another allowance: Task2 failed/rejected/fallback and report/capture regressions.
- A normal answer at a budget ceiling or forced closing is not resumable: Task1 original cap, refusal/truncation/closing/default path tests.
- A harness note must not be retained as owner evidence: Task2 actual transcript/capture coordinates and source-withdrawal tests.
- Later provider/capture failure must retain already accepted diagnostics: Task1 record callback and Task2 nonzero CLI report tests.
- Model completion improvement is separate from reasoning: Task3 fixed lexical paired controls, independent artifact/lineage audit, first results retained.

### Task1: Generic per-run completion observer

**Files:** `mecha-core/src/agent.rs`; new focused `mecha-core/src/agent_completion.rs` and tests (declared/reexported from agent.rs). Minimal explicit None initializer changes in `mecha-core/src/subagent.rs` are also owned here; children do not inherit completion. No native/CLI/docs edits.

**Interfaces:** `agent::{CompletionObserver,CompletionReadiness,CompletionArtifactIssue,CompletionFeedback,CompletionObservation,CompletionReport}`. Async trait `observe(&self)->CompletionObservation`; synchronous default `record(&self, report:&CompletionReport)` receives accepted snapshots only. Readiness Ready/Incomplete/Unavailable; artifact issue Missing/Malformed; feedback fields `required_read_missing:bool`, `artifact:Option<CompletionArtifactIssue>`, `do_not_repeat_read:bool`. Observation has readiness/optional feedback. Report fields are `first` and `latest:Option<CompletionObservation>`, `feedback_delivered:bool` (appended), `timed_out:u32` and `interrupted:u32`; enums serialize snake_case. Use serde on wire types. `RunContext::with_completion_observer(Arc<dyn CompletionObserver>)->Self` and `with_completion_deadline(tokio::time::Instant)->Self`; None defaults. Native supplies its single original 180-second deadline, shared with the existing cancellation timer, before provider discovery. Native implements callback with a task-local report cell. Loop owns report/allowance; no async observer side effects or RunOutcome schema changes.

- [x] Write and run RED behavioral regression: normal nonempty answer without output ends before corrective tool call on old behavior (new API no-op scaffold allowed to demonstrate real assertion failure).
- [x] Implement normal-EndTurn-only observation and one fixed registered user harness notice; continue existing loop without budget reset. Record accepted report at each transition so provider Err cannot erase it. Reject invalid/empty feedback; bounded timeout and pre/post deadline/cancel checks.
- [x] Cover repair continuation, no second feedback, no-control legacy path, all terminal exclusions, first/final report, provider failure after feedback, timeout/cancel/late result, unchanged taint and same/reused context allowance. No model calls.
- [x] Run scoped agent tests/strict core Clippy, freeze patch/report with actual RED/GREEN output, independent task review. No commit/push.

### Task2: Public artifact readiness and native/CLI integration

**Files:** `mecha-core/src/memory_situated_workflow.rs`, `memory_experience_bridge.rs`, focused new `memory_native_completion.rs` and native/bridge tests; minimal lib module declaration; `mecha-cli/src/commands/memory.rs`, `mecha-cli/tests/memory_cli.rs`.

**Interfaces:** `NativeCompletionRequest { rows:usize,keys:Vec<String> }::into_config()->Result<NativeCompletionConfig>` (validated fields private), reexport from native workflow. Add `run_native_session_with_completion(database,storage,authority,request,recall:Option<NativeRecallConfig>,completion:NativeCompletionConfig,root,factory,cancel)->Result<TaskReport>` with existing factory bounds. `TaskReport.completion` optional omitted when absent, containing `protocol`, `shape_digest` and `report:CompletionReport` only. Observer is fresh per task and stores only loop-accepted reports synchronously. Existing check/recall fields supply remaining diagnostics.

- [x] Write RED regression for missing output action plus actual capture, and public-shape-only acceptance of semantically wrong values.
- [x] Factor existing safe read minimally; add gold-independent readiness, strict shape validation, fixed output path and required-read semantics from spec. Preserve old artifact/envelope behavior and old wire bytes.
- [x] Wire same agent RunContext, capture and optional bounded1MiB `--completion-request`. Validate before directory/provider; new v2 checkpoint/error identity. Nonzero unsatisfied completion; wrong semantic answer alone retains existing outcome behavior. Preserve diagnostics on run/capture errors.
- [x] Cover malformed/duplicate keys/nonstring/wrong rows, symlink/nonregular/oversize, wrong/unknown valid values, consumed recall, cancellation/ceiling, source withdrawal, encrypted passthrough, no-overwrite/preflight and feedback exclusion from owner evidence. Run covering feature core/CLI tests and strict Clippy. Freeze patch/report for independent review.

### Task3: Fresh bounded completion comparison and publication

**Files:** new focused comparison orchestration/test and authored fixture/protocol under `prototypes/memory/`, reusing existing native-vector driver helpers without altering its frozen fixtures/results. Root owns `CHANGELOG.md`, architecture/design/README/handoff/history, CLI docs and Site.

**Interfaces:** actual feature-enabled CLI with Task2 completion argument; feedback-off/control and feedback-on run identical task/public shape/lexical recall/model settings, separate state copies. No new executor or extra recall. Record memory enabled and adaptive learning frozen in this component protocol; the broader system suite uses separate explicit settings and primarily enables both. Four fresh paired cases: two positive supported choices and two unknown controls (irrelevant evidence and withdrawn decisive evidence); exact domains and observed-method-only selection explicit. Freeze eight planned follow-ups, counterbalance order and retain all failures/missing results. Independently capture authored seed episodes first.

- [x] Prepare fixture/protocol and independent review before inference. Test authored workflow/no-replay/isolation/hidden-gold exclusion and independent artifact/recall/capture scoring; preserve original vector experiment.
- [x] Integrate reviewed Task1/2 patches; run fmt, both all-target builds, default/feature workspace suites, standalone/Python checks, strict all-target/all-feature Clippy and actual MSRV on isolated Spark checkout. Review combined diff before scored calls.
- [x] Run one frozen eight-session comparison on already resident authorized model, no service switch. Audit first artifacts/lineage/request parity and intervention timing independently. Separate completion, supported correctness/unknown controls and costs. If blocked retain actionable evidence, no manufactured benefit.
- [ ] Root performs handoff/source-grounded docs, strict Spark source/message privacy, normal-hook commit/push to existing draftPR, final CI/review loop, and private Site publication with actual outcome boundaries.
