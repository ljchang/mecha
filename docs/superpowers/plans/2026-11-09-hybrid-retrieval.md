# Bounded Hybrid Retrieval Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a bounded, opt-in semantic/lexical evidence selector and measure it on synthetic data without changing memory authority or durable storage.

**Architecture:** Harden the existing embedding transport and add a checked mixed-input batch operation. Prepare eligible original passages synchronously, embed outside SQLite transactions, then revalidate and reuse the existing context assembler. A thin experimental CLI compares lexical, dense and RRF modes; vectors live only for that invocation.

**Tech Stack:** Existing Rust workspace, rusqlite/SQLCipher, reqwest/Tokio and the existing local embedding service. No new dependencies or database schema.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md` §31.18.

## Global Constraints

- Synthetic-only, default-off `memory-prototype`; no production activation, private import, graph/nightly migration, persistent vectors or chat-model inference.
- Preserve existing lexical/persona APIs, canonical records, taint, current access/forgetting rules and complete source/dependency bundles.
- Eligible corpus: at most256 records,262144 original-field bytes,512 passages; each passage at most2000 Unicode scalars with exact UTF-8 offsets. Hidden/future/ineligible rows do not consume these bounds.
- Checked embeddings: at most8000 scalars per formatted input,16 inputs/request,262144 serialized request bytes,8388608 response bytes,64 attempts,120 seconds total,dimensions1..=8192. No silent checked-input truncation or partial result.
- Numeric loopback HTTP, no credentials/query/fragment, no redirects/proxies; explicit expected alias and host-declared revision. Alias is not weight attestation.
- RRF k=60,one-based ranks; f64 cosine, one max passage score/revision; existing sequence/revision tie-breaks. No semantic confidence or decision-benefit claim.
- Prior baseline4fd8ed71 passed all18 CI jobs, docs and review; runtime source unchanged since that evidence. Do not repeat clean-baseline full suites merely for a new task boundary. Run covering checks on new changes and the final combined required gate.
- Root owns shared docs, PR/Site and integration. Workers do not commit/push: exact final staged source/message must first pass strict Spark privacy scans. Use frozen patches/review packages instead of worker commits. Preserve existing worktrees and receipts.

## Review Focus

- A source is forgotten or its scope changes during embedding: revalidate the entire eligible corpus and never return stale dense results.
- Ineligible/future rows alter ranks, input text or caps: filter before statistics, passages and eligible budgets.
- A long Unicode field has relevant evidence at its end: cover exact bounded spans without silent truncation or duplicate RRF votes.
- An embedding server redirects, lies about indices/model, returns huge bodies or overflows f32: refuse without leaking input or exposing raw server diagnostics.
- A semantic-only anchor brings a derived concept/relation: complete authorized provenance still fits together or is withheld under the unchanged packet budget.

### Task 1: Checked embedding client

**Files:** modify `mecha-core/src/embed.rs`; add a focused sibling test module if needed; preserve existing callers/tests.

**Interfaces:** Add public immutable `EncodingProfile` (validated constructor and stable `id()`), `EmbeddingInput { text:String, task:Task }`, and `EmbeddedBatch { profile_id:String, vectors:Vec<Vec<f32>>, ... bounded cost metadata }`. Add `Embedder::embed_checked(&self, inputs:&[EmbeddingInput], profile:&EncodingProfile) -> Result<EmbeddedBatch>`. Profile fields cover every identity component in §31.18; expose read-only accessors needed by the workflow. Keep legacy `embed/query/recall_query` signatures and response compatibility. Mixed inputs carry query instructions exactly once; passages remain bare. The new checked path accepts Query/Passage only; Recall stays on its existing API. No crate-level provider trait is needed.

- [x] Write RED tests:307/308 spy target receives zero requests; subprocess proxy receives zero; oversized checked input refuses before send; shuffled indices reorder; duplicate/missing/fractional indices, wrong/missing model, mixed dims,zero norms and1e40 refuse; second split half failure returns no partial batch; streamed cap and server-echo errors are bounded/sanitized; all profile components affect identity; legacy fixture lacking model/index still works.
- [x] Run focused tests against old behavior and preserve meaningful runtime failure evidence (new-symbol compile failure alone is insufficient where old behavior can be exercised).
- [x] Implement shared no-redirect/no-proxy transport and additive checked operation. Preserve whole-operation bounds during overflow splitting; never infer exact artifact revision from echoed alias.
- [x] Run `cargo test -p mecha-core --lib embed::` plus relevant persona search embedding tests and strict scoped Clippy. Record environment skips/failures separately; no full suite yet.
- [x] Freeze patch/postimages/report for independent spec+quality review. Root applies only after clean review.

### Task 2: Prepared hybrid evidence/context selection

**Files:** add `mecha-core/src/memory/hybrid.rs` and focused tests; update `memory/mod.rs`, `memory/recall.rs`, `memory/context.rs` minimally to share candidate selection, anchor/neighbor budgeting and assembly. Standalone harness must still compile the same storage module without importing core's HTTP embedding module.

**Interfaces:** In `memory::hybrid`, expose `HybridMode::{Lexical,Dense,Hybrid}`, `HybridOptions { profile_id:String, dimensions:usize, required_mention:Option<String> }`, opaque `PreparedHybrid`, `HybridVectors { profile_id:String, query:Vec<f32>, passages:Vec<Vec<f32>> }`, and `HybridResult` carrying the unchanged `ContextPacket`, requested/effective modes, typed fallback reason, ranked-anchor diagnostics and bounded counters. `PreparedHybrid` exposes read-only query and exact passage inputs/identities, plus readiness; it does not accept caller-edited corpus/policy. Add `Memory::prepare_hybrid_context(&ContextRequest,&ContextPolicy,&Situation,&HybridOptions)->Result<PreparedHybrid>` and `Memory::finish_hybrid_context(&PreparedHybrid,Option<&HybridVectors>,HybridMode)->Result<HybridResult>`. The same prepared/vector data may be compared across modes, but every finish revalidates independently. ContextRequest.bindings must be empty on this explicit route. Fingerprints bind original bytes and full eligible ranking corpus, including allowed nonmatching records needed for BM25 statistics. Mention filtering restricts dense anchors before embedding/top-k.

- [x] Write tests that fail on lexical-only retrieval of a semantic-only source with authored vectors; pin RRF order, distinct score labels, equal budgets, max-per-revision collapse and stable ties.
- [x] Cover all five Review Focus cases. Add exact Unicode spans/all-fields/late-field cases, corpus overflow explicit fallback before embedding, malformed vector error, empty/literal-unmatched corpus success without network, historical snapshot/current restrictions, required mentions (including every lexical-fallback path), corpus addition/removal/original-and-indexed-bytes/scope/external-authority changes between prepare/finish, and same-width profile mismatch.
- [x] Implement the two-phase route. No SQLite transaction survives preparation; finalization reconstructs/rechecks the full eligible view. Bounds/embedding absence/corpus changes return freshly computed lexical fallback with typed reason; no arbitrary prefix of the corpus is embedded.
- [x] Reuse existing whole-record limits, neighbor authorization and complete context assembly. Ordinary lexical methods retain their exact behavior and score interpretation. Never attach RRF numbers to `RecallHit.bm25`.
- [x] Run covering recall/context/hybrid/native regressions and standalone shared-source tests; record old-behavior failures and new passing results. Freeze patch/report for independent spec+quality review before root integration.

### Task 3: Core comparison workflow, thin CLI and frozen experiment

**Files:** add `mecha-core/src/memory_hybrid_workflow.rs`, its tests and a synthetic fixture under `mecha-core/src/memory/fixtures/`; expose feature-gated module in core; add one `memory evaluate-hybrid` command in `mecha-cli/src/commands/memory.rs` and CLI contract tests. Root updates prototype README, architecture/handoff/history, changelog, relevant user reference and Site.

**Interfaces:** Core workflow owns fixture ingestion, preparation, checked embedding calls, mode comparison and scoring. CLI supplies explicit loopback endpoint, alias, host-declared artifact revision and dimensions; no global config or implicit provider/model selection. Use the existing experimental acknowledgement, new-path suffix/exclusive-create policy and synthetic-only marker. Start with plaintext synthetic evaluation fixtures; refuse encrypted selection for this command rather than adding an untested path. Other encrypted operations remain unchanged. No owner stores or private source data are discovered.

- [x] Freeze at least16 synthetic cases spanning paraphrases, exact identifiers, multi-evidence retrieval and time/scope/withdrawal distractions, plus separately reported empty-gold cases. Separate public query/policy fields from host-only gold/forbidden IDs. Record fixture/protocol hashes before the first embedding call. Existing viewed retrieval cases may be additional development data, never relabeled holdout.
- [x] Add authored-provider or local HTTP tests for mixed input construction, shared query/passages across all three modes, whole-operation timeout, explicit fallback accounting, no network on bound failure, no writes during embedding, unchanged filters/budgets, and refusal/no-overwrite CLI behavior. Grading must not call the retrieval implementation as its own oracle.
- [x] Implement a core report with planned/completed/fallback/error denominators, complete evidence coverage, reciprocal rank, returned evidence bytes, unauthorized returns and wall-clock/call/input/output costs. No reasoning/decision score. For nonempty gold, MRR uses first relevant returned anchor (only anchors present in final packet); complete coverage requires all gold anywhere in packet. Missing relevant anchors score0; empty-gold metrics are null and returned counts separate. Semantic fallback/error rows score0 in their planned nonempty-gold denominator. Time ordinary lexical context independently; report shared preparation/embedding once, per-mode finalization separately, and label any per-arm total with shared cost as derived. Distinguish lexical fallback output from successful dense/hybrid treatment.
- [x] Run focused core/CLI tests, freeze patch and receive independent review. Root integrates after clean review.
- [x] Run final fmt, workspace build/tests, all-target/all-feature Clippy, relevant full feature/standalone qualification. Reuse verified baseline evidence only where source/config identity justifies it; label all scopes.
- [x] On Spark, verify configured artifact/response identity and runtime dimensions before the frozen synthetic comparison. Activating the existing on-demand embedding service for these requests is authorized; do not restart/reconfigure it or switch chat models. Record actual identity evidence, complete first results and costs. Do not tune thresholds/cases after seeing results.
- [ ] Root completes strict staged/message privacy scans, one integrated PR publication, exact-head CI/review loop and private Site update. Report any quality non-improvement honestly; a mechanically correct feature need not win every retrieval case.

## Deferred work

Persistent/encrypted vector caches, production GoalMemory integration, persona cache-identity migration, corpus-scale indexing, downstream decision trials, new concept/motif discovery, owner graph migration and external-file/full-disaster recovery remain outside this increment. Existing three minor review notes remain tracked separately unless a change directly depends on one.
