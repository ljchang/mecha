# Connected bridge model comparison implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development task by task with independent reviews.

**Goal:** Qualify the existing episode–execution–artifact–appraisal–recall bridge with a bounded real-model paired comparison.

**Architecture:** A feature-gated core trial runner uses ordinary `Agent::run_in`, native session recording, `LocalAuthority` and `GoalMemory`. A strict local provider and thin example run one trial per process. A Python coordinator freezes synthetic cases and runs paired fresh memory/empty-store arms, retaining all failures.

**Tech stack:** Existing Rust workspace dependencies, SQLite, Python standard library. No new dependency, production configuration or schema.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31 and the approved continuation from the connected appraisal bridge. This plan records implementation choices within the user's standing authorization to progress, use subagents, and update draft PR #846 and the owner-private Site.

## Global constraints

- Existing worktree and branch; baseline `256693e23` has all CI checks successful and unchanged code qualified on isolated Spark (6302 default / 6649 feature tests, zero failures).
- Synthetic Test sessions only, default-off `memory-prototype`; no production services/stores/nightly jobs or mecha-graph changes, no merge.
- Reuse ordinary dispatch, path jail, conversation taint, Clean-only learning and existing capture limits. No alternate executor, inferred owner approval, code-mode capture or ambient owner configuration.
- Four cases × two replicates: 8 planned A runs and 16 planned B runs, not 24 model responses. No adaptive retries, selective replacement or exclusion.
- Each pair runs A once, then freezes its completed files. Memory B receives closed copies of A database/authority; control B gets an empty valid v6 store. B input files, prompt, criterion, gold, tools and model budgets are identical. A originals/gold/audit remain outside B's jail; no prior.json answer copy.
- B runs in distinct OS processes and directories. Authority copied per arm retains A's immutable original transcript/artifact paths and never mutates A. Withdraw only copied authority in withdrawal case.
- Same frozen B task gold across arms. Score evidence-appropriate abstention separately. Information availability is not appraisal-specific causal benefit.
- Real model: explicit numeric-loopback endpoint/model/holds directory; no proxies, redirects, automatic load/switch, fallback or inference retry. Fast mode, temperature0.6, explicit seed. Preserve started calls, failures, unknown usage and actual returned model.
- Bounds: max_turns8, per-call max_tokens4096, total output16384, whole trial600s, provider request180s. Capture refusal remains a measured result.

## Review focus

1. A's artifact or host gold accessible in B: jail/adversarial reads and actual provider-audit canaries must fail to leak.
2. Failed A capture or B transport/parse failure: every planned slot remains in denominator; no success-only conditioning.
3. Source withdrawal and cloned authority: verify original A hashes unchanged and actual GoalMemory returns no withdrawn evidence.
4. Model changes/cancel/timeouts: hold shared router, check resident identity, no retries; preserve partial native records and request-start audits.
5. More than16 passages/32 calls, wrong artifact and missing usage: retain independent artifact check and distinct capture/error status without widening production contracts.

## Frozen interfaces

`memory_bridge_trial::{TrialRequest, TrialSource, TrialConfig, TrialReport, run_trial}`; serde DTOs deny unknown fields.

Request JSON:
```
{version:1, task_id:string, prompt:string, criterion:ConfirmedCriterion,
 expected_rows:[{string:string}], files:{flat_filename:utf8_text},
 source:null|{database:absolute_path,authority:absolute_path,confirmation:sha256},
 withdraw_source:bool}
```
Reserve `output.json`, require bounded flat input names/content; root is a NEW absolute directory. Runtime creates root/workspace, native transcript, authority, memory-prototype SQLite, provider-audit.jsonl and report.json. Host files and gold never enter workspace or prompts. Source-less control always has GoalMemory too.

```
pub async fn run_trial(root:&Path, request:TrialRequest,
 provider:Box<dyn Provider>, config:TrialConfig,
 cancel:CancellationToken) -> Result<TrialReport>
```
`TrialConfig` fields: `max_turns:u32`, `max_tokens:u32`, `max_output_tokens:u64`, `timeout_secs:u64`; Default has fixed bounds above. Runtime reports configuration. Its recording wrapper applies fast/local-only request settings before recording and delegates `wire_body`, recording effective request and response fields. All auditing is host-only, request-start precedes await, and partial records survive cancellation.

Report v1 minimum frozen keys: `version`, `task_id`, `status`, `run_error`, `artifact_check`, `capture_receipt`, `capture_error`, `confirmation`, `database`, `authority`, `transcript`, `artifact`, `provider_audit`, `turns`, `model_calls_started`, `model_calls_completed`, `input_tokens`, `output_tokens`, `elapsed_ms`, `recall_calls`, `recall_references`, `taint`, `config`. Optional errors/usage/receipt serialize null when unavailable. ArtifactCheck uses existing verdict names. Runtime/transport failure is data after initial structural validation; do not silently return success without retained report.

Add narrow host API `LocalAuthority::check_artifact(&self, confirmation:&str)->Result<ArtifactCheck>` using existing frozen checker. It grants no model authority.

The executable starts its absolute 600-second cancellation deadline before provider initialization; native cancellation gets a bounded two-second drain to preserve partial streaming records. Coordinator615s is a shutdown allowance, not another inference budget.

Provider module `memory_trial_provider`: `LocalTrialConfig { endpoint:String, model:String, holds_dir:PathBuf, seed:u64 }`; `LocalTrialProvider::new(config, cancel).await -> Result<Self>` implements Provider, holds router across trial lifetime, checks resident before inference, validates response model, enforces request timeout, and uses strict local transport without fallback. Model fast mode is applied by runner recorder; provider must preserve exact request auditing.

Example command:
```
memory_bridge_trial --synthetic-only REQUEST_JSON NEW_DIRECTORY --endpoint URL --model MODEL --holds-dir ABSOLUTE_DIRECTORY --seed U64
```
The example parses explicit inputs, constructs local provider, invokes runner. No ambient setup/global loads. Missing feature refuses.

## Tasks and ownership

### 1. Native single-trial runner
Files: create `mecha-core/src/memory_bridge_trial.rs`, associated `_tests.rs`; narrow edit `memory_experience_bridge.rs` + tests for host checker. Parent owns `lib.rs` declarations.
- [x] Add failing tests for completed/wrong artifacts, host-only gold, copied source/empty control/withdrawal, partial-error recording, distinct artifact/capture verdict and existing capture limits.
- [x] Implement frozen runner interfaces, bounded directories/inputs, native record and authoritative capture. Never replay uncertain execution.
- [x] Verify actual GoalMemory references/taint and original A immutability, then independent task review.

### 2. Local provider and thin executable
Files: create `mecha-core/src/memory_trial_provider.rs`, associated tests; `mecha-core/examples/memory_bridge_trial.rs`. Parent owns declarations.
- [x] Write failing localhost-server tests for strict endpoint, resident mismatch/no inference, returned model mismatch, cancellation, actual seed/temperature/fast request, no redirect/retry.
- [x] Implement exact provider/config and CLI interfaces, request timeout and lifetime model hold; preserve wire_body delegation.
- [x] Test example argument/feature gating and independent task review.

### 3. Frozen paired evaluation
Files: create `prototypes/memory/bridge_model_trial.py`, `prototypes/memory/tests/test_bridge_model_trial.py`.
- [x] Build four deterministic cases: prior calibration suffix, repeated quoted-newline structure/new records, changed inclusion/transformation criterion, withdrawn prior parameter. Use exact `[{'status':'needs_information'}]` string-row abstention. Never put hidden B parameter in B criterion text.
- [x] Freeze manifest/source/binary/helper hashes/model controls/budgets/case oracle+input hashes/8 pair IDs/alternating B order before inference; refuse existing output or modified frozen inputs.
- [x] Invoke native executable once per slot with explicit paths and provider settings, preserve stdout/stderr/exit/timeouts. A capture failure marks both planned B slots not_started and remains in denominators; admitted but failed A artifacts may continue, transparently.
- [x] Snapshot A hashes before either B and verify after both; assert identical B prompt/files/criterion/gold/settings/tools; treat no-retrieval treatment as a result.
- [x] Add scripted HTTP preflight and tests for real native capture/recall, errors, malformed/missing artifacts, no-memory calls, unknown usage, no gold/A-artifact leaks, source withdrawal and immutable A. Scripted evidence is not real model evidence.
- [x] Report all planned8pairs/24slots, task correctness, evidence-appropriate abstention, capture coverage, actual recall refs, usage/latency and errors. Independent task review before real inference.

### 4. Parent integration and publication
- [x] Run focused/integration tests and whole-change independent review; resolve major/medium findings.
- [x] Transfer exact isolated checkout to Spark, verify numeric-loopback resident model/settings, compile and scripted-qualify exact binary, freeze manifest, run bounded real comparison once; retain failed attempts without retuning.
- [x] Run required workspace fmt/build/strictclippy/default+feature tests for changed source. Update architecture/usage/handoff/history and owner-private Site with measured outcomes and limits.
- [ ] Actual-owner-corpus privacy scan before push; update existing draft PR846, read automated review and CI. Do not merge.

## Recorded outcome

All 24 planned attempts and 112 model calls finished; protocol integrity passed
but the all-runs-completed gate did not (three exhausted runs). Later exact
artifacts were 6/8 versus 4/8; the two gains supplied a missing parameter and
controls appropriately abstained. Two captures exceeded the fixed passage limit.
Native todo checks exposed an unavailable-shell contract mismatch, and one pair
exposed ambiguous output-field wording. Preserve the frozen run; the next work
is capability-aware planning-check admission and bounded capture reliability,
not an adaptive rerun or broad interpretation expansion. HISTORY/HANDOFF record
the evidence and full checks; PR #846 carries current publication/review status.
