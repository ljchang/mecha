# Outcome recall and situated reappraisal plan

> Execute with subagent-driven-development; each worker has a separate existing managed worktree. Parent owns docs, integration, model runs, PR and Site.

**Goal:** Measure whether one retained actual outcome improves later decisions under changed goals.
**Architecture:** Read-only core preparation of a source-only/source-plus-outcome pair, real bounded recall, explicit effective-measurement policy, shared arithmetic checks, thin CLI and frozen independent pilot. No new persistence schema or executor.
**Tech Stack:** Existing Rust/SQLite/serde/CLI plus standard-library Python pilot; no new dependencies.
**Spec:** docs/MECHA-MEMORY-DESIGN.md section27.

## Global Constraints

- Default-off memory-prototype, explicit synthetic input/domain/scopes; owner memory and production remain untouched. No schema change, stored-source rewrite, action execution, automatic adoption or production integration.
- Both arms retain identical original source evidence; only SourceAndOutcome adds the actual recalled observation. No EvidenceView::None substitution.
- Real Memory::recall at a frozen snapshot, limit100, neighbors0, max_bytes100000. Exact operational observation revision and bytes must appear or preparation fails; no hidden direct-lookup fallback.
- Same workload only; validate fixture/state/choice provenance via existing action/model/read accessors. Never use fixture truth to prepare/check requests. Refused/unknown observations never provide numbers.
- Explicit observed-over-expectation policy for current arithmetic; retain original bytes/hashes and distinct visible kinds. Exclude prior verdicts/recommendations/gold from prompts. Existing prompt/schema bytes unchanged.
- Source invalidation aborts the pair with no newly exposed source-dependent audit/reply, preventing further calls; ordinary model failures remain rows. Recheck complete current dependencies, exact access and cancellation before inference, after provider awaits and after calls. No transaction spans inference. Returned artifacts remain private/untrusted/advisory/executed:false.
- One call per arm, no repair/retry; failed replies stay failed; usage missingness remains null. The pilot has five pairs at seeds7,19 =20 calls, counterbalanced; no resident-model switch or autoload.
- Parent integrates and publishes one coherent update to draftPR846 and owner-private living Site; no merge or production deployment.

## Review Focus

- Recall miss, wrong workload, forged observation, foreign store/access, stale/restricted/forgotten sources cannot become usable actuals.
- Source revision/revocation/forgetting during a provider await must stop dispatch; after inference must refuse stale results and further calls.
- Tight/relaxed budgets must re-evaluate measurements without copying historical failure into current policy.
- Unknown/refused observations and hidden alternative quality must not be fabricated or leak oracle information.
- Counterbalanced paired arms must differ only in the observation intervention, with costs/failures and both evidence/hidden-oracle scores separated.

### Task 1: Core paired preparation, evaluation and CLI

**Files:** new mecha-core/src/memory_outcome_reappraisal.rs and tests; lib.rs feature export; minimal crate-only helpers in memory_action_runtime.rs, memory/decision.rs, memory/experience_checks.rs as needed; mecha-cli/src/commands/memory.rs and CLI/API tests.
**Interfaces:** Arm {SourceOnly,SourceAndOutcome}; prepare_pair(db:&Memory,action:&ActionReceipt,current:&Contract,query:&RecallQuery,access:&Access)->Result<PreparedPair>. PreparedPair exposes request(Arm), recheck(db,access), serializable audit, and opaque OutcomeGuard for a canonical read-only reopened source check. evaluate_pair accepts explicit model identity, arm order and a callback receiving prompt/schema/OutcomeGuard; returns per-arm calls, advisory decision and assessment with raw preparation audit. Exact ergonomic signatures are worker-owned and must be reported before runner implementation.
Finalized actions remain terminal and cannot be cancelled through the supported API; reject already-cancelled/unfinalized input, and test live invalidation through source revision/revocation/forgetting.
Core resolves the action's model/read lineage (no caller-supplied raw observations). Require an uncancelled finalized action; match raw recall to the exact retained observation. Preserve read original text and expectation projections; source-only sees three source passages, treatment additionally sees the actual record. Shared numeric checker should consume one effective method entry and provenance per method without presenting duplicate actual/expected entries as independent methods. All visible required comparison sources remain cited. No mutable Memory access or database writes.
CLI name evaluate-outcome-recall, under memory --experimental --db. Required --synthetic-input --domain --scope --action --contract JSON --query --model, --seed default7, --outcome-first flag. --snapshot JSON supplies the original exported Snapshot and is mandatory for inference; only initial prepare-only may omit it and use the current snapshot. --prepare-only returns the exact frozen pair without provider setup. The pilot must supply the first snapshot to all subsequent preparations/runs and compare raw recall plus snapshot as well as requests. Otherwise explicit --endpoint and --holds-dir required, reuse strict local resident-model client; add an opaque guard variant or equivalent minimal callback to recheck after awaited discovery. Keep stdout JSON and no implicit production config.
- [ ] Write failing tests for pair contrast, source preservation, authentic real recall, missing/fake/foreign evidence, same workload, refusal/unknown and access/revision/forget/cancellation before/during calls; count callbacks and verify zero writes.
- [ ] Implement module/helper/CLI with old prompt bytes unchanged; test quality7 budgets4,10,2 and quality9 budget4 expected choices from spec. Exclude hidden Quill quality and old Failed from prompt. Assert actual controls come from the host, not model text.
- [ ] Run focused core/API/CLI tests and strict lint; self-review; commit assigned files and report API, exact commands/results and limits. No real model calls or deployment.

### Task 2: Independent pilot and qualification

**Files:** prototypes/memory/outcome_reappraisal_trial.py, test_outcome_reappraisal_trial.py; optional mecha-core/tests or mecha-cli/tests new independent qualification files; .github/workflows/ci.yml to execute the standalone harness and Python test discovery on the existing pinned Ubuntu leg. No product module edits.
**Interfaces:** Consume Task1 CLI prepare-only/run JSON and existing retained mismatch database from prior durable_action_trial; explicit --binary --db --action --endpoint --model --holds-dir --output. Runner must not execute any new action or reconstruct the saved source/outcome. Shared output manifest and per-call progress artifacts.
- [ ] Freeze five cases and two seeds from spec, independent source-supported and simulator oracles; reject malformed/duplicate responses and distinguish correct model abstention from host withholding.
- [ ] Use prepare-only for every pair before model calls; record database/source/observation hashes, raw recalls, prompts, contracts, model settings and planned order. Refuse existing output directory; record incomplete matrices on failures. Runtime-prepared requests must equal frozen requests or stop. Exclude oracle fields from model input.
- [ ] Add regression tests for contrast, hidden-information diagnostic, missing data, failed/truncated replies, usage missingness, partial runs and no action writes. Scripted actual-CLI test proves pairing/guards/reporting with zero real inference; pause /models, revoke/forget a source in a copied database before returning discovery, and assert zero completion requests, failed exit and no source-bearing report. No worker real model calls.
- [ ] Add pinned-Ubuntu CI steps for cargo test --locked in prototypes/memory and python3 -m unittest discover -s prototypes/memory -p 'test_*.py', with MECHA_SESSION_KIND=test. Run these checks and self-review; commit assigned files and report. Parent runs at most20 real calls only after independent review.

### Task 3: Parent integration and publication

- [ ] Baseline workspace build/tests; review each task spec and quality; integrate then independent whole-increment review.
- [ ] Required fmt/strictClippy/workspace tests and isolated release; verify installed artifact; run frozen20-call resident-model pilot and independently audit source/evidence and explanation support.
- [ ] Update architecture/changelog/design and repository handoff, retain current evidence boundaries. Update existing draftPR846, read automated review through no major/medium findings.
- [ ] Publish living Site with results, limits and next step; verify content/layout and deployed version. Preserve production and owner-store separation.
