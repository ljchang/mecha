# Constraint Application and Abstention Implementation Plan

> For agentic workers: use superpowers:subagent-driven-development.

**Goal:** Measure and improve source-grounded constraint checking and genuine abstention.
**Architecture:** An opt-in response protocol over the existing read-only paired recall path. Host validation and independent grading remain separate.
**Tech Stack:** Rust core/CLI, SQLite, Python experiment harness.
**Spec:** User request and MECHA-MEMORY-DESIGN.md section28.

**Current protocol:** the corrective addendum below supersedes the original v2
48-slot schedule. Core v3 / runner v2 use 24 corrective responses; the stopped
original run remains separate.

User-authorized increment, 2026-11-06. Base 41c6bce18. Spec: improve application of current hard constraints and genuine model abstention, with source-linked outcome memory, independent grading and no new action execution during comparison.

## Global constraints
Preserve legacy outcome-reappraisal-v1 byte behavior/defaults and prior results. No new persistence schema, executor, production wiring or dependency. New v2 diagnostic explicitly selects response treatment. Authenticated retained actuals remain the only override, numeric same-workload policy unchanged. Reuse pre/post/discovery/final source guards and local resident-model holds. No source, expected answer or hidden oracle leakage. User has authorized autonomous implementation, isolated Spark testing, draft PR and Site updates; do not merge.

## Experiment
Freeze four new configurations A-D (from prior next-diagnostic-design), seeds31/43, source-only/source+outcome, and three treatments: choice-first no thinking; feasibility-first no thinking; feasibility-first with2048 reasoning tokens (8192 total). All v2 treatments reason cap4000 Unicode characters. 48 first responses maximum; no repair, retries or adaptive replacement. Explicit feasible iff energy<=budget AND quality>=minimum; choose fastest feasible, else abstain. Keep request source bytes fixed across treatments, compare visible-evidence correctness and required abstention; simulator optimum separate. Fresh synthetic action lineages for Arden and Quill, created with authored model responses, never caller-authored actual insertions. Counterbalance and freeze before inference. Capture complete requests/settings/artifact hashes/usage; record incomplete matrix and preserve DB/settings if interrupted. Reasoning itself is not evidence.

## Task 1: core and CLI
Owner worker core, isolated worktree. Add diagnostic response policy/options API alongside prepare_pair legacy default, protocol v2 metadata and checks parser. Feasibility-first schema has candidate_checks for all3 methods before choice: numeric effective values and energy_ok/quality_ok/feasible, citing applicable supplied evidence. Host recomputes all claims and withholds inconsistent report without replacing raw choice. Bound parsing, reject duplicate/missing/foreign entries, preserve original provenance. Expose explicit CLI response mode and bounded reasoning budget, frozen preparation and model controls accurately reflect settings. Test old default equivalence, exact-boundary, empty-feasible and unnecessary-abstention cases, override arithmetic, malformed/false checks, Unicode and reasoning bounds, existing source invalidation guarantees.

## Task 2: frozen runner and independent rubric
Owner worker pilot, separate worktree. Add standalone constraint_abstention_trial.py + tests reusing existing pilot shared functions where valid, no rewrites of old scoring. Explicit freshcases/fixture creation protocol, 48slot frozen matrix, independent source/oracle rubric, model choice and host withholding separate, check accuracy and citations separate, strict input/preparation validation under python-O. Fixture generation via installed CLI durable action/authored HTTP provider; all real inference only parent execution on Spark. Tests include wrong arithmetic, all feasible/infeasible/ties/boundaries, unnecessary abstention, tampered metadata and interruption. Interface coordinated with Task1; no tracked docs edits.

## Task 3: integration, qualification and experiment
Parent integrates reviewed worker commits. Required baseline then focused regression/independent reviews. fmt, strict Clippy, workspace build/default/allfeature tests, standalone and Python. Install isolated release and verify digest and new CLI behavior. Scripted endpoint checks before48call frozen real comparison. Independent result audit, no efficacy claim beyond observed authored development cases.

## Task 4: documentation and delivery
Parent updates existing design/handoff/history/changelog/prototype guide and living Site preserving audience/anchors. Commit/push draftPR846 coherently, poll CI and automatedreview and fix verified major/medium findings. Record implementation vs tests vs model findings separately. Next steps determined by frozen diagnostic; no additional adaptive model runs thisincrement.

## Corrective protocol after delivery failure

The first run stopped with 22 completed responses and two uncertain dispatch slots.
Actual CLI schema output listed choice/citations/reason before candidate_checks;
reordering only `required` did not deliver the intended checks-first protocol.
Preserve that incomplete v2 run and make no efficacy claim from it. Correct the
serialized properties order and version the diagnostic core protocol as v3.
Qualify actual HTTP property order explicitly; sorted JSON exports cannot prove it.

Use a separate, frozen 24-response corrective diagnostic, keeping total possible
inference within the original 48-call ceiling. Reuse authenticated source/actual
lineages, but use new contracts E=(3,9), F=(1,9), G=(6,8), H=(6,9), where tuple is
energy budget/minimum quality and stores are AB, AB, C, D respectively. One new
seed59, both evidence conditions and all three treatments produce24 responses.
Expected source/outcome answers are abstain/Arden, abstain/abstain, Quill/Arden,
Quill/abstain. This is a one-seed corrective development check, not the originally
planned replicated comparison, a replacement for missing rows, or a holdout.
