# What makes a harness good at solving tasks

Research pass, 2026-08-19, prompted by the question: is it planning? the loop?
persistence? Why are Claude Code, Codex, the Qwen and DeepSeek harnesses good
at *coding* specifically?

**Venue key**: ✅ peer-reviewed · 📄 preprint · 📰 vendor/blog · 🔮 folklore.

Read against this project's existing passes, which already own half the
answer: `VERIFICATION-RESEARCH.md` (verifiers), `CONTEXT-RESEARCH.md`
(distraction and compaction), `PRIOR-ART-RESEARCH.md` (three harnesses read
line by line), `BENCHMARK-RESEARCH.md` (what a harness score means).

---

## The one-sentence answer

It is not the loop — every harness has the same loop — and it is barely
planning. **What separates harnesses is the observation half of the loop: does
the model's intent land in the world deterministically, does the result come
back honestly, and does the record of its own failures get out of the way
before it conditions the next step.** Coding harnesses look brilliant because
coding is the one domain that hands them a free sound verifier.

---

## 1. Calibrate first: how much is a harness worth?

Two measurements that disagree in an instructive way.

📄 **Terminal-Bench 2.0** ([arXiv:2601.11868](https://arxiv.org/abs/2601.11868),
ICLR 2026) ablates it directly: Codex CLI's resolution rate moves **52%**
swapping GPT-5-Nano→GPT-5.2, while Gemini-2.5-Pro moves **17%** swapping
OpenHands→Terminus 2. Their conclusion: *model selection is usually more
important than agent scaffold*.

📄 **Claw-SWE-Bench** ([arXiv:2606.12344](https://arxiv.org/html/2606.12344v1)),
350 issue-resolution instances, 8 languages, identical prompts and timeouts,
disagrees: with the model *fixed* at Qwen 3.6-flash the five harnesses span
**66.0% → 38.6% (27.4 pp)**, against a **29.4 pp** spread across nine models.
Comparable.

Both are true, because the harness spread is not symmetric. Read the harness
column: OpenClaw 66.0, hermes 62.6, zeroclaw 58.3 — a 7.7 pp band across three
serious harnesses — and then nanobot 47.4, GenericAgent 38.6. **The spread is
bad harnesses being bad, not good ones being brilliant.**

📄/📰 The same shape appears from the other end: **mini-swe-agent** is ~100
lines with no planner, no subagents, no memory, and scores **>74% on SWE-bench
Verified**; on SWE-bench Pro one comparison puts it at **68.3%** with
Claude-Opus-4.7 against Claude Code's **62.2%** and hermes-agent's 64.6%
([mini-swe-agent](https://github.com/swe-agent/mini-swe-agent),
[SWE-bench Pro scaffold analysis](https://docs.bswen.com/blog/2026-04-20-swe-bench-pro-agent-scaffold/)).
Treat the exact ranking as 📰 — but the qualitative claim replicates: feature
count does not predict score, and structure can *cost* headroom (the same
comparison has mini gaining +11.5 pp from a model upgrade where Claude Code
gains +6.1).

**The design consequence.** The ceiling on cleverness is a few points. The
floor under broken plumbing is thirty. Spend accordingly.

---

## 2. The largest measured single lever is plumbing

Same paper, same backbone, one variable — whether the harness's edits actually
apply:

| Claw-SWE-Bench adapter | Pass@1 | patch-application failures |
|---|---|---|
| bare adapter | **19.1%** | 69.1% |
| full adapter | **73.4%** | <1.5% |

**54 points from making the edit tool work.** No planning, no memory, no
verification — the model was already producing the right answer and the harness
was dropping it on the floor.

This is not an outlier, it is the genre. 📰 Edit-format measurements put
search/replace at **70–80%** exact-match reliability on evolved codebases, and
`apply_patch` at **50%+ failure** for some models that were not trained on it;
Codex trains on its own format, Cursor spends a second fine-tuned model purely
on *applying* the first model's sketch
([edit formats](https://www.morphllm.com/edit-formats),
[how harnesses edit files](https://wuu73.org/aiguide/infoblogs/coding_file_edits/agents.html)).

The generalisation, and it is the most useful sentence in this document:
**a harness's first job is to be a lossless channel between intent and world,
and to fail loudly when it isn't.** mecha already has one instance of the rule
written down — `docs_replace` reports zero matches as a failure, because a
model told "ok" describes an edit that never happened. That rule is the whole
of section 2.

---

## 3. The loop is not the differentiator; the observation half is

📄 The [Dive into Claude Code](https://arxiv.org/html/2604.14228v1) analysis
puts a number on where the engineering actually goes: **1.6% of the codebase is
AI decision logic, 98.4% is operational infrastructure** — "a thin reasoning
layer wrapped in thick operational infrastructure", one `queryLoop()` shared by
CLI, SDK and IDE. Every harness surveyed here interleaves reason → act →
observe. Nobody wins there.

Where the good ones differ is what `observe` returns:

- **Ground-truth environmental feedback at every step**, not model self-report.
- **Exact search over retrieval.** Claude Code deliberately has no index and no
  embeddings: grep/glob is exact where vector search is approximate, auditable
  where retrieval is a black box, zero build cost, and it improves for free as
  models write better queries
  ([Claude Code doesn't use RAG](https://harrisonsec.com/blog/agent-retrieval-cost-curve-claude-code-grep-vs-rag/)).
  This project's `CONTEXT-RESEARCH.md` §1 supplies the mechanism the blogs
  don't: a *better* retriever made RAG output worse (Cuconasu, SIGIR 2024),
  because retriever-ranked near-misses are the maximally damaging distractor.
- **A truncation policy with an opinion.** 📰 Codex truncates the *middle* of
  large tool responses; Claude Code keeps and re-projects. Both are decisions;
  the harness that has none returns 200 KB of `npm test` output into a 32k
  window and loses the run. mecha's `ToolsConfig::resolved_output_budget`
  exists because of exactly that failure.
- **Graduated compaction rather than one summary.** The Claude Code analysis
  reports **five sequential context shapers** before each call (budget
  reduction, snip, microcompact, context collapse, auto-compact), on the
  principle that "no single compaction strategy addresses all types of context
  pressure."

---

## 4. Planning: real, but not for the reason people say

📄 [The Long-Horizon Task Mirage?](https://arxiv.org/html/2604.11978v1) is the
best failure taxonomy available. Seven categories; **72.5% of failures are
process-level** (environment interaction, instruction following, planning,
history accumulation) against 27.5% design-level (memory limits, catastrophic
forgetting, false assumptions). Their emphasis: *early subplanning deviations
propagate through later actions and convert recoverable local mistakes into
irreversible trajectory-level failures*, and **"model scaling alone is unlikely
to resolve the dominant failure mechanisms."**

So planning failures dominate. That does **not** license a planner module —
`VERIFICATION-RESEARCH.md` already settles that intrinsic planning-and-critique
is worth ~15 points where an external sound verifier is worth 48 (Blocksworld,
identical loop: 40% none → 55% LLM self-critique → **88%** sound verifier), and
that self-critique is negative on several benchmarks.

The resolution: **a plan's measured value is as anti-drift memory, not as
reasoning.** The todo list works because it is re-read every step and keeps a
constraint alive that would otherwise be summarised away — the "catastrophic
forgetting" and "history error accumulation" rows of the taxonomy, not the
"planning error" row.

Which makes the mecha-specific corollary sharp: `Tool::carried_state` is not a
nicety. A todo list that reaches the model only through the echo in its last
tool result is a plan that a compaction deletes — and compaction happens
precisely on the long runs where the plan is load-bearing. That mechanism *is*
the measured half of planning.

---

## 5. Persistence: the naive version is measured negative

✅ **[Measuring Long Horizon Execution in LLMs](https://arxiv.org/abs/2509.09677)**
(ICLR 2026) is the key paper and it reframes the question:

- Failures on long tasks are **execution, not reasoning** — models fail steps
  they can demonstrably do in isolation.
- **Self-conditioning**: models become *more* likely to err when the context
  contains their own prior errors. It is not a long-context effect and
  **it does not go away with model scale.**
- **Thinking mitigates it**, and sequential test-time compute extends
  single-turn executable horizon substantially.
- **Marginal single-step accuracy gains compound into exponential gains in
  task length.**

📄 The [FSM execution study](https://arxiv.org/html/2511.14777v1) reports the
same negative self-conditioning as a decaying per-turn accuracy curve.

Two consequences, and the second is uncomfortable for this codebase:

1. **The last bullet is why section 2 is the highest-leverage work in a
   harness.** Shaving per-step failure — an edit that lands, a result that
   isn't silently truncated, an error message that says what actually happened
   — buys horizon *superlinearly*. Boring plumbing is not a consolation prize;
   it is the exponential term.
2. **"Keep retrying" is not persistence, it is self-conditioning.** The
   harness lever is getting the record of failure *out of the context* before
   the next attempt — or restarting the attempt from a clean conversation.
   mecha's `evict_superseded_results` deliberately exempts errors ("a failed
   call says nothing about the target, and 'what failed' is what stops it being
   retried"). That reasoning is sound for *one* error and inverts for a
   pile of them: three failed attempts at the same call are a distractor
   corpus written by the model about its own incompetence. **Open question
   worth measuring here: cap repeated-failure retention (keep the newest, fold
   the rest into a count), and see it in pass^k.**

---

## 6. Why they are good at *coding* specifically

This is the part that generalises worst, and it is the answer to the user's
last question.

Coding hands the harness a verifier that is **executed, sound, cheap,
adversarially independent of the producer, and able to say no** — the four
properties `VERIFICATION-RESEARCH.md` names as the conditions under which a
verification loop earns its cost. The compiler and the test suite are that,
for free, in every repository. ✅ Self-Debugging isolates it: with test
execution, MBPP 61.4 → **70.8**; the identical loop with self-critique only,
61.4 → **57.6** (negative). Spider, where no unit tests exist: 81.3 → 81.3,
literally zero.

So "why is Claude Code good at coding" decomposes into:

1. The domain supplies a free oracle, and the harness runs it.
2. The state is text in files — cheap to snapshot, diff, and revert, so an
   error is recoverable rather than terminal.
3. Search is exact (§3), so retrieval is not a source of near-miss distractors.
4. Everything is reversible under version control, which is what lets the
   permission system default to *act* rather than *ask*.

**None of the four hold for a personal assistant.** There is no oracle for "was
that the right reply to this email", mailbox state is not snapshot-and-revert,
and sending is irreversible. That is the honest reason mecha cannot import
coding-harness results wholesale — and it is also the argument that the outbox
is not a safety feature bolted onto an agent, it is **the substitute verifier**:
a human review gate is the only oracle available in this domain, and it happens
to satisfy all four properties (executed, sound, independent, cheap "no").

---

## 7. The four harnesses, read for mechanism

**Claude Code** — thin reasoning / thick ops (1.6% / 98.4%), one loop across
all front-ends, five-stage compaction, deny-first permissions where "a broad
deny cannot be overridden by a narrow allow", subagents with isolated context
returning only summaries into a sidechain transcript so they cannot inflate the
parent. Two measured numbers from the analysis worth keeping:
users approve **93% of permission prompts** (approval fatigue is real, and it
is the empirical case for mechanical policy *ahead* of the human — mecha's
interlock → hook → approver ordering, arrived at independently), and
auto-approve rates rise from ~20% at <50 sessions to **>40% by 750** (trust
ramps whether or not you design for it).

**Codex** — `apply_patch`, a diff format the model is *trained on* rather than
prompted into; middle-truncation of tool output; execution-policy sandboxing;
📰 tuned for sustained unattended autonomy where Claude Code is tuned for fast
feedback. The trained-format point is the transferable one: co-designing the
tool surface with the model beats prompting a general model into a format.

**Qwen Code** — the living open fork of the Gemini CLI lineage. 📰 The
reported failure mode is dialect: its tool-calling API needs translation, and
multi-turn agentic loops break at the seams. Relevant here because mecha's
`provider/openai.rs` is exactly that translation layer.

**DeepSeek (dsh)** — surveyed in this repo 2026-08-16 (`HISTORY.md`); mecha
took `recall`, the cache lens, and the Landlock backend from it. Strong on
context mechanics, **no taint tracking, no send-sink concept, no summary
validation.**

📄 One component-level ablation exists
([AHE](https://arxiv.org/html/2604.25850v3), Table 3), on a minimal seed:
memory **+5.6 pp**, tools **+3.3**, middleware **+2.2**, system prompt
**−2.3** alone. The finding to keep is the interaction: the three positive
gains sum to +11.1 pp and the full stack delivers **+7.3**, because
"memory, middleware, and the system prompt all push toward the same
closure-style verification, so stacking them spends turns on redundant
re-checks." **Harness features are not additive, and overlapping ones bill you
turns for nothing.**

---

## 8. Verdict against mecha

Checked against the source, not assumed. Three things this pass expected to
find missing are already built, and saying so is the point of the section:

- **The edit tool already fails loudly.** `fs_edit` returns an error on zero
  matches *and* on ambiguity ("`old` appears {n} times"), which is §2's rule
  implemented before §2 was read.
- **Per-call reliability is already captured.** `ToolCallTrace` carries
  `is_error` / `denied` / `unknown` / `staged`, `RunOutcome` carries
  `malformed_tool_args`, and `Scorecard` already reports `tool_errors`,
  `unknown_tools` and `malformed_tool_args`.
- **The executed-verifier slot already exists twice**: `expect.verify` at eval
  time, and `post_tool` hooks at run time.

Also already right, and now with citations behind it: staleness-aware eviction,
`carried_state`, `recall` over compacted history, pass^k over pass@k, interlock
ahead of approver, the outbox as this domain's substitute oracle, tool output
budgets derived from the window.

### What is actually missing

1. ~~**Nothing prunes a pile of identical failures.**~~ **Built 2026-08-19**
   as `compact::collapse_repeated_failures`, to the spec below. Verified before
   building: eviction skips
   errors by construction (`compact.rs:400`, `if *is_error || ...`), and
   `thin_old_results` only truncates the *tail* of long results outside the
   recent window — a 60-character error message is never touched by either
   pass. So eight failed attempts at the same call survive verbatim, forever,
   which is precisely the corpus §5 says degrades the next attempt. The rule
   the exemption protects ("what failed is what stops it being retried") is
   satisfied by the *newest* error alone. **Collapse older same-target errors
   into a count, keep the newest verbatim.** Deterministic, unit-testable in
   `compact.rs`, gradeable by pass^k. `StopCause::Loop` is not this: it stops a
   run that already went wrong, and it is dormant until a compaction.
2. **A run can stop `Completed` with its trailing tool calls errored, and
   nothing notices.** That is the silent-failure shape — 75.8% of
   self-assessing AppWorld runs are false successes, and no LLM judge exceeds
   AUROC 0.65 at catching it, where the deterministic signal is free. A
   `completed_over_failures` flag on `RunOutcome` plus an `expect` check is the
   harness-grading category the eval rig already has slots for
   (`stop_cause`, `taint`, `blocked_sends`).
3. **Unattended runs' reliability is invisible.** The numbers exist per run and
   are aggregated only by `eval`. §5's compounding result says a trigger
   quietly erroring on a third of its calls is a degrading harness, and today
   nothing reads it. This is a marker plus a doctor check — the shape doctor
   was designed for — not a new subsystem.
4. **Do not add a planner or an in-run critic.** §4 and §6 both refuse it, and
   the AHE non-additivity result (§7) prices the overlap: mecha already runs
   four mechanisms that push toward re-checking yourself (learned rules,
   carried state, summary validation, loop guard). Measure that stack with
   `--ab-rules` and pass^k before adding a fifth.
