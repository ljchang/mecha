# Budget finalization notice implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development. Implementation and review remain separate; parent owns qualification, shared documentation and publication.

**Goal:** Test whether a truthful early turn-budget notice improves completion without blocking useful late evidence.

**Architecture:** Default-off per-run advice in the ordinary Agent loop, enabled explicitly by the bounded memory trial. Preserve every execution guard and ceiling. Compare identical tasks with and without the notice in a separately versioned protocol.

**Tech Stack:** Rust core/trial, Python experiment coordinator, SQLite memory, existing Site.

**Spec:** User-approved next increment; detailed source-grounded design is `results/memory-planning-checks-20261107/stopping-next-step.md` (local evidence). This plan records the durable requirements below.

## Global constraints
- Default-off `RunContext::budget_notice: bool`, builder `with_budget_notice(bool)`; no persistent Config or new production default.
- Effective turn ceiling is the per-run Budget override or AgentConfig max_turns. Deliver once before a request with two remaining turns, or one for a one-turn run. Zero-budget run sends none.
- Use valid appended text and classify the exact message as harness voice. Preserve once-per-run state through compaction. Never classify it as owner correction or external evidence.
- Advice cannot add calls, change tools/order/permissions, extend timeout, fabricate evidence or write a fallback artifact. Existing other stops win.
- TrialConfig adds serde-default false `budget_notice`; old v1 requests deserialize unchanged. Existing frozen comparison cases/results remain untouched.
- No owner data, production deployment, model load/switch, graph migration or extra inference retries. Bound real experiment to 4 cases x 2 seeds x 2 arms (16 tasks) plus any explicit source setup; retain all attempts.

## Review focus
- Effective override and 0/1/2-turn edge cases; cancellation and exhausted budgets.
- Tool pairing, taint, source restrictions and owner/harness attribution.
- Compaction or context-overflow retries must neither repeat a notice nor consume a new task turn invisibly.
- A justified late read must remain possible; early abstention is a failure on solvable tasks.
- Protocol gold must never enter requests; failed/missing artifacts stay failed.

### Task 1: Opt-in loop notice and trial seam
**Files:** `mecha-core/src/agent.rs` and focused tests, `mecha-core/src/memory_experience_bridge.rs` and tests, `mecha-core/src/memory_bridge_trial.rs`, its tests, native example argument surface if needed.
**Produces:** `RunContext::with_budget_notice(bool)` and `TrialConfig::budget_notice` default false. Fixed singular/plural notice text recognized by `is_harness_voice`.
- [ ] Read relevant architecture and prove pre-change baseline before production edits.
- [ ] Add failing request-capture regression: eight calls, notice first present request7 exactly once, permitted late read/write, no ninth call.
- [ ] Add effective override/short-cap/default-off, budget/cancel precedence and provenance/owner-text tests. Exclude recognized harness text from new UserRequest passage admission while keeping raw transcripts and real owner corrections. Verify old behavior fails the new positive assertion.
- [ ] Implement minimal advice after stop checks and before request construction; do not alter boredom or forced-final-answer machinery.
- [ ] Thread optional native example `--budget-notice true|false` flag through bounded trial, record it, and test hidden-gold isolation plus ignored-notice MaxTurns without artifact repair.
- [ ] Run targeted tests and report commands/results; independent reviewer checks spec and quality. Parent commits only after qualification/privacy scan.

### Task 2: Separately frozen comparison
**Files:** new `prototypes/memory/budget_notice_trial.py`, focused tests; reuse existing bridge coordinator utilities without modifying its frozen cases or protocol.
**Consumes:** Task1 TrialConfig bool. **Produces:** preregistered manifest, request/response audit and complete denominator report.
- [ ] Cases: unavailable parameter, directly available parameter, initial miss then recoverable authorized clue, withdrawn/restricted source. Freeze two seeds, balanced arm order, 8 turns, same model controls and tool set.
- [ ] Tests assert paired request equality except notice option, all failed slots retained, hidden gold absent, exposure measured from actual wire, solvable abstention scored as failure.
- [ ] Run authored native preflight, then one fixed real comparison with no retries. Report exposure, correct/appropriate abstention, premature abstention, missing artifacts, caps, calls/tokens/latency and actual source-guard outcomes separately.

### Task 3: Qualification and delivery
- [ ] Independent increment review, repair major/medium findings, strict build/fmt/clippy and serial workspace suites on Spark.
- [ ] Diagnose GitHub contradictory mergeability; repair through supported reversible PR operations if needed. Never weaken CI or merge the PR. Verify actual nonempty check results.
- [ ] Update HANDOFF/HISTORY/architecture/changelog and existing research plan as warranted; preserve previous evidence. Follow handoff skill. Update PR846 and private Site with measured results and limitations.
