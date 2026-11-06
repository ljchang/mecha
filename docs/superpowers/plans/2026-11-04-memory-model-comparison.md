# Memory model comparison implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Measure lexical, bounded goal-aware, and oracle retrieval with the same local answering model on new synthetic scenarios.

**Architecture:** Core owns the fixture, evidence assembly, prompts, deterministic grading and async experiment orchestration through an injected completion callback. The existing Mecha CLI supplies a tool-less, local-only Provider connection using explicit endpoint/model arguments; no global configuration, connectors or production memory are loaded. The independent memory harness compiles the same core code.

**Tech Stack:** Existing Rust, SQLite FTS, serde, Provider and QuarantinedPass; no new production dependencies.

**Spec:** `docs/MECHA-MEMORY-DESIGN.md`, `docs/AGENT-MEMORY-SYSTEMS-RESEARCH.md`, and the approved three-condition comparison in this conversation.

## Global constraints

- Synthetic inputs only; fresh `.memory-prototype.sqlite` database, no production deployment replacement.
- Same answer prompt, model, sampler, token limit and 8,000-byte evidence budget in each arm.
- Goal-aware retrieval adds at most two bounded literal searches; no model-selected scopes, time filters, paths or endpoints.
- Gold choices/citations are grading-only; oracle evidence is deliberately privileged and labelled.
- Measure structured decision correctness and citations, abstention, retrieval coverage, elapsed time and reported tokens. Do not label these free-form factuality or executed planning outcomes.
- New cases are held out from the previous 32-case diagnostic, authored before model runs; no tuning against observed answers. This small authored set is not an independent leaderboard benchmark.
- Delay private-data migration and nightly jobs. Preserve the default-off feature and existing no-model commands.

## Review focus

- Malformed/oversized query plans cannot expand access or exceed two searches; count failure rather than silently substituting a successful arm.
- Missing/wrong citations, guessed answers and malformed responses never pass a grounded check.
- Merging multiple searches cannot exceed the shared evidence budget or introduce out-of-scope/time records.
- Oracle-only gold and answer choices cannot leak into query planning; changed-goal pairs must share episodes and question.
- Provider/model mismatch, truncation, timeout and missing responses remain failures with usage/error evidence; no remote fallback.

### Task 1: Core comparison and fixture

**Files:** create `mecha-core/src/memory/model_evaluation.rs`, `model_evaluation_tests.rs`, and `fixtures/model_comparison.json`; export module from `mod.rs`.

**Interfaces:** `run(path, complete, progress) -> Future<Result<Report>>`, callback `FnMut(String) -> Future<Result<ModelReply>>`; `ModelReply` carries text, reported input/output tokens and model. Report records each arm's prompt/response/evidence/queries, metrics and errors, fixture hash and limitations.

- [x] Write failing tests for bounded plans, grounding/abstention, no gold in prompts, paired goals, evidence budgeting and callback failures.
- [x] Run the standalone harness to observe failure, then implement the runner. Fixed 16 cases across new scenarios, interleaved/counterbalanced arms, literal initial question, same final evidence cap. Use the existing authorized recall/read paths.
- [x] Run standalone tests and inspect fixture/gold independently of model answers.

### Task 2: CLI and Spark experiment

**Files:** modify `mecha-cli/src/commands/memory.rs`, dispatch in `main.rs`, tests in `mecha-cli/tests/memory_cli.rs`; update prototype README and existing architecture/design/history/handoff documentation.

**Interfaces:** `mecha memory --experimental --db NEW evaluate-model --endpoint http://127.0.0.1:8080 --model ID --holds-dir ROUTER_HOLDS --seed 7`. Explicit loopback endpoint, model and synthetic fixture; no configuration discovery. Timeout per request; no tools, no retries/fallbacks. Keep model calls out of synchronous database methods.

- [x] Add failing CLI rejection test for nonloopback endpoint, proving no DB is created.
- [x] Implement thin async provider adapter and existing command dispatch; structured answer prompts, bounded output, model verification.
- [x] Verify targeted tests, required workspace checks and fresh code review; fix important findings with regression tests.
- [x] Transfer verified source to the isolated Spark prototype checkout and build before inference; use the resident model without changing production services. Record served model, binary/source provenance and environment. Run once with a fixed seed, retain machine-readable results, inspect failures and summarize paired comparisons. No post-result fixture/prompt optimization.
- [x] Run handoff skill and commit verified code/docs. Leave branch unmerged.


Completed: implementation `d3ca03b82`; full verification 5,930 workspace tests,
33 standalone tests, seven CLI tests, strict lint/format checks. Isolated Spark
run completed 48 rows/64 calls, no errors; results and limitations in
`docs/AGENT-MEMORY-SYSTEMS-RESEARCH.md` §7. All important review findings fixed;
per-field usage missingness remains the documented minor. Branch unmerged.
