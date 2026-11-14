# Required-mention memory retrieval plan

> **For agentic workers:** use superpowers:executing-plans inline.

**Goal:** Recover facts about named alternatives without increasing the search/evidence budget, and measure retrieval separately from reasoning.

**Architecture:** Add `Memory::recall_mention(query, mention, access)` beside unchanged `recall`. A bounded literal token phrase gates anchors before top-k; descriptive terms rank within it. Use a non-stemming unicode61 ephemeral mention index alongside the existing eligible-only BM25 index, so identities are not conflated by stemming. Context neighbors remain independently authorized and need not contain the mention. This is lexical mention matching, not entity resolution.

**Tech stack:** existing Rust, rusqlite FTS5, serde; no dependencies/schema migrations.
**Spec:** user-approved continuation of research §8's next bounded experiment.

## Constraints and review focus

- Preserve every account/session/snapshot/time/access filter, forgetting, taint and byte cap. An invalid/empty mention errors, never falls back to broad recall.
- Mention max256 bytes/32 alphanumeric tokens; quoted literal phrase, no executable FTS operators; case/diacritic/punctuation normalization follows unicode61, no alias inference.
- Original search and both model comparisons remain unchanged.
- Diagnostic replays all12 frozen evidence-informed plans (same initial + <=2 searches, two anchors/search, four final records/8000 bytes). Explicit manually annotated mentions must occur in their saved query and initial visible text; gold used only for coverage. Null annotations keep the original query. Report annotations/source hashes and per-query anchors, not just aggregate gains.
- This already-seen fixture is development evidence, not held-out validation. No new model calls, answer scores, private imports or production deployments.
- Add duplicate-valid-citation regression deferred from prior review while testing the existing grading boundary.

## Task 1: Required mention API

Files: memory/recall.rs, recall_tests.rs.
- [x] RED regression with an irrelevant high-ranked record; mention gate recovers named facts. Phrase/stemming/operator bounds, absent mention, caps, time/current restrictions/snapshot/foreign account tests.
- [x] Implement shared private recall path; ordinary recall stays unchanged. Standalone suite GREEN.

## Task 2: Reproducible retrieval diagnostic

Files: memory/model_followup.rs (small visibility changes only), memory/mention_evaluation.rs + tests, fixtures/mention_replay.json, main memory CLI and CLI tests.
- [x] Save12 existing evidence-informed plans plus explicit mention annotations and hashes; no gold in retrieval inputs.
- [x] RED diagnostic/CLI tests, implement `evaluate-mentions`, expose trace and coverage only. Run no-model comparison and save local JSON artifact.
- [x] Full fmt/clippy/workspace and standalone tests, fresh independent final review, fix Important findings with regressions. No merge/push.
- [x] Update existing research, architecture, README, changelog, history and scoped handoff; commit branch and preserve it.

Completion: required-mention replay recovers complete support10/10 vs6/10 with manual bindings; all12 baseline packets exactly reproduced. No model answers rerun. Reviewed Unicode normalization bug fixed RED→GREEN. Full workspace5944 passed/25 ignored, standalone45, CLI9 and strict lint/fmt passed. Source and measured limitations recorded in research§9; branch left unmerged, Spark unchanged.
