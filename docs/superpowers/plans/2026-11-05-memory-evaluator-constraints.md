# Memory evaluator constraints and conservative revision implementation plan

> Use superpowers:executing-plans inline, with one fresh final code review.

**Goal:** Reduce the observed evaluator errors and unnecessary revisions without
expanding the synthetic prototype into live planning.

**Architecture:** The host determines which checks apply. Models distinguish
original evidence from provisional concept citations; host validation resolves
both against the same opaque passage receipt. The revision controller preserves
an existing recommended candidate and otherwise requests one minimal repair with
explicit guards, followed by reassessment.

**Tech stack:** Existing Rust core and CLI, SQLite prototype, Python calibration.
**Spec:** Approved discussion and `docs/HANDOFF.md` memory findings; base50500f70b.

## Constraints

- Keep personal mecha-graph knowledge independent; no migration or private imports.
- No actions, tools, new storage, automatic chat integration or production restart.
- Preserve scope/freshness/forgetting checks across all calls and fixed context.
- Host provenance checking never establishes semantic entailment.
- Preserve old calibration scripts, artifacts and measured results.

## Tasks

- [x] Baseline build/workspace tests. Add RED tests for host-owned no-concept
  handling, typed citation categories and skipping revision of an adequate plan.
- [x] Implement in `plan_assessment.rs`: two model checks without concepts,
  three with concepts, host-added not-applicable check labelled host-derived;
  original `citations` and `concept_citations` vocabularies; explicit evaluation
  protocol and revision-status fields. Keep the public merged citation report.
  Known blockers take priority in the prompt; revisions must repair demonstrated
  defects, avoid unsupported timing and preserve valid conditional branches.
- [x] Add a v2 calibration runner reusing the frozen six cases, tolerating a
  legitimately skipped revision and separating that from failure. Add grader
  regression tests. Leave the original runner/protocol untouched.
- [x] Run focused tests, actual CLI mock and required formatting/clippy/workspace
  tests. One fresh read-only final review; fix important findings with RED/GREEN.
- [x] Install separately on Spark, replay the frozen cases once with the resident
  model/seed7, retain failed rows and inspect actual citations/revisions. Compare
  descriptively with the prior run; these reused cases are development data.
- [x] Update existing docs, handoff and append-only history; commit, no merge.

## Review focus

- A model cannot supply a host-generated not-applicable judgment.
- Typed citation categories must be enforced by host code, not just a schema.
- An adequate candidate skips revision even when another candidate is inadequate;
  no-revision status must distinguish this from revision not being requested.
- Skipping revision must still run final source authorization/freshness checks.
- Calibration must include failures, preserve unknown usage and avoid claiming
  that skipped revisions or model self-approval prove real-world improvement.

## Verification

Standalone memory tests and Python grader tests; main workspace all-feature tests
and strict clippy; actual CLI integration; Spark source/model/seed/usage/latency
recorded with case-level artifact review. No policy-improvement claim without
examining the plan itself against the predeclared rubric.

Completed at source `ea16b5304`; qualification and remaining semantic limitations are recorded in `docs/HISTORY.md`.
