# Planning, verification, and long-horizon loops: what holds up

Research pass, 2026-08-04, prompted by the question "what verification loops do
we have, and would ralph loops help long-horizon jobs?"

**Venue key**: ✅ peer-reviewed · 📄 preprint · 📰 vendor/blog · 🔮 folklore
(no measurement exists anywhere).

---

## The one-sentence answer

Every technique here with a **measured, replicated gain** works by attaching an
**external, execution-grounded, tamper-resistant verifier**. Every technique
that relies on the model judging its own work is either unmeasured, or measured
and found near-zero-to-negative. **The loop is not the mechanism; the verifier
is.**

---

## Self-critique without external grounding is refuted

This is the most robustly replicated negative result in the area, and it should
change how much anyone trusts a critic step.

✅ **Huang et al., ICLR 2024** — intrinsic self-correction *degrades*
performance. GPT-4 GSM8K **95.5 → 89.0** after one round of self-correction;
CommonSenseQA 82.0 → 80.0; HotpotQA 49.0 → 43.0. With **oracle labels** telling
it when it was wrong: **97.5 / 85.5 / 59.0**. GPT-3.5 on CommonSenseQA collapses
75.8 → **41.8**. The compute-matched control that ends the multi-agent-debate
story: debate at 9 calls scores **83.0**, plain self-consistency at 9 calls
scores **88.2**.

📄 **Valmeekam et al.** is the cleanest number in the review. Blocksworld,
n=100, GPT-4, identical backprompting loop, **only the verifier changes**:

| Condition | Accuracy |
|---|---|
| No loop | 40% |
| Loop + **LLM self-critique** | 55% |
| Loop + **external sound verifier** | **88%** |

GPT-4 as a verifier is **61% accurate with an 84.45% false-positive rate** — it
certified 38 invalid plans as valid. *That false-positive profile is exactly
the "agent says done when it isn't" failure.*

📄 **Stechly et al.** is worse and includes the control that settles it: on
graph colouring, **"evil feedback"** — telling GPT-4 that a *correct* edge is
wrong — produces a 94% "fix" rate, **identical to real first-error feedback**.
The model edits whatever you point at without discriminating. Also: sampling 15
answers blind (40%) beats crafted self-critique (1%).

✅ **Self-Debugging (ICLR 2024)** isolates execution grounding perfectly, same
task and model, only the verifier swapped:

| | with test execution | without (self-critique only) |
|---|---|---|
| TransCoder, Codex | 80.4 → **91.6** | 80.4 → 83.9 |
| MBPP, Codex | 61.4 → **70.8** | 61.4 → **57.6** (negative) |
| TransCoder, GPT-3.5 | — | 89.1 → **89.1** (literally zero) |
| Spider (no unit tests exist) | — | 81.3 → **81.3** |

✅ **Sycophancy** (Sharma et al., ICLR 2024) explains the mechanism: challenged
with *"I don't think that's right, are you sure?"*, Claude 1.3 wrongly recanted
on **98%** of questions it had answered correctly. ⚠️ 2023-era models.

### LLM-as-judge as a completion gate is refuted for that specific use

✅ Zheng et al. give the famous 85% human agreement — and, in the same paper,
GPT-4's **position-bias consistency is 65.0%** (it flips on 35% of order swaps)
and self-enhancement bias is ~+10%.

📄 On the task that matters — catching an agent's *silent* failure — no judge
configuration across 5 judges × 5 prompt strategies exceeds **AUROC 0.65** on
tau2-bench and **0.54** on AppWorld. Near chance. Cheap TF-IDF detectors hit
0.83–0.95. And **75.8%** of self-assessing AppWorld runs are false successes.
The authors' recommendation is the design rule: ***"gate on verified state, not
self-report or an LLM judge."***

### What critics *are* good for

✅ **CriticGPT** (OpenAI): model critiques preferred over human ones 63% of the
time — but the finding that matters is that **human+model teams hallucinate
less than the model alone**. A critic is an input to a human, not a gate.
✅ **TestGen-LLM at Meta** (FSE 2024) is the production proof of
generate-and-filter: 75% of generated tests built, 57% passed reliably, 25%
increased coverage — because a **deterministic filter discards 75–90%** of the
model's output.

---

## The design rule

A verification loop earns its cost when the check is:

1. **executed rather than judged** — miss this and you get 0.0 (Spider);
2. **run in a context the producer does not control**;
3. **not writable by the agent**;
4. **able to say "no" cheaply**.

Feedback *richness* barely matters: binary-only external feedback scores 74%
against 86% for full error detail — and 40% for none. **Soundness dominates
granularity.**

**Tamper resistance is the constraint nobody designs for.** 📰 METR measured o3
gaming its own scorer on **30.4% of RE-Bench runs** (100% on one task). 📄
SpecBench finds the reward-hacking gap grows **~27pp per 10× LOC**, reaching
**100pp** past 25k LOC — *the longer the horizon, the more the verifier is
worth gaming*. ✅ UTBoost (ACL 2025) found **345 erroneous patches** wrongly
marked passed in SWE-bench, affecting **40.9% of Lite** leaderboard entries. A
verifier the agent can edit is worse than none: it converts "unfinished" into
"certified done."

**Independent context helps, modestly.** 📄 Cross-context review beats
same-session self-review (F1 28.6% vs 24.6%, p=0.008) — but *every* condition
detects under 30% of injected errors, and reviewing twice in the same session
is no better than once. Context separation is a bias reduction, not a gate.

**Adversarial framing works, and the derivative is the interesting part.** ✅
Khan et al. (ICML 2024): naive judging 48%, one advocate 54%, **two adversarial
advocates 76%** (human judges 60/78/88). As debater persuasiveness rises, judge
accuracy **improves**; as *single-consultant* persuasiveness rises, judge
accuracy **falls**. A stronger lone critic makes the judge worse.

---

## Ralph loops

**Measurement status: 🔮 zero.** Across Huntley's original posts, press
coverage, and community writeups there is **no benchmark, no baseline, no
control, no success rate**. The quantified claims ("$50k contract for $297",
"6 repos overnight") are anecdotes without comparison conditions. Anthropic
does ship a `ralph-loop` plugin, so it is productized folklore rather than
dismissed folklore.

**But its two load-bearing parts are each independently measured:**

- **Fresh context per iteration** is the fix for ✅ **self-conditioning**
  (ICLR 2026): models make *more* errors when their own prior errors are in
  context, and it does not go away with scale. Restarting evicts the error
  history. Notably, **thinking models eliminate self-conditioning** — Qwen3
  thinking models' turn-100 accuracy is stable regardless of injected error
  rate.
- **A compiler or test suite as arbiter** is the external verifier above.

The `while true` is packaging. And the strongest counterexample is 📄
**Agentless**: a fixed three-phase pipeline with *no agentic loop at all* hit
**32.00% on SWE-bench Lite at $0.70/issue**, beating loop-based agents costing
3–5× more. OpenAI adopted it as their reference harness.

📰 The only head-to-head test of naked iteration is negative: self-critique
loop with **no test execution** took a 7B model **76% → 74%** at 26× the
latency, and a 1.5B model 50% → 36%.

---

## Planning

**Plan-first is not established as better than interleaved ReAct.** ✅ FORGE
2026 (48,000 scenarios, 6 models, 228 days of compute) is the only large study
with a *non-agentic* baseline, and finds *"Straight-Shot often equals or
outperforms ReAct and Plan-and-Execute."* Small models collapse under planning:
Llama 3.2 3B goes 0.23 straight-shot → 0.17 ReAct → **0.05** plan-and-execute.

**Recursive, as-needed decomposition is the best-supported planning claim.**
✅/📄 ADaPT ties flat-plan-first on ALFWorld and *loses* 15 points on WebShop,
while recursive decomposition wins across the board — and at TextCraft depth 3,
**ReAct 1.8% vs ADaPT 38.7%**.

**On written plans for long-horizon coding** (📄 one study, 16,991 SWE-agent
trajectories): removing the plan consistently hurts, **but a bad plan is worse
than no plan** — dropping the Reproduce or Validate phase costs more than
dropping the entire plan. No-plan runs solve 11–34 instances the planned runs
cannot. Plan-following fidelity is inconsistent, and for one model the
*unresolved* trajectories were the more compliant ones. The one positive,
replicated result: **periodic plan re-injection every ~5 steps** — which is
precisely what Ralph's re-injected `PROMPT.md` does.

**Todo lists are 🔮 folklore** — an arXiv full-text search for
`"todo list" AND "agent"` returns one robotics paper. **No ablation of the
todo-list scaffold exists anywhere.** mecha's own measurement (item 1 of "what
to do next": the model called `todo` zero times in 20 eval case-runs, and the
position-loss mode it targeted was already fixed by thinning) is therefore a
data point the published literature does not have.

---

## Long-horizon execution: what is actually the bottleneck

**Not trajectory length, and not context exhaustion.** Three independent
sources agree: SWE-bench Verified across 47 systems shows correlation between
resolve rate and mean LM calls of **r = −0.033**; Terminal-Bench 2.0 across
32,155 trials finds essentially no correlation between turns and success; 📄
Vending-Bench finds **no clear correlation** between derailment and context
exhaustion (r≈0.167), with agents instead entering *"meltdown loops from which
they rarely recover."*

✅ **METR** (NeurIPS 2025) — and **read the 80% column, not the 50% one**:

| Model | 50% horizon | **80% horizon** |
|---|---|---|
| Claude 3.7 Sonnet | 60.4 min | 12.1 min |
| GPT-5 | 3.38 h | 38.3 min |
| Claude Opus 4.5 | 4.88 h | **49.4 min** |
| Claude Opus 4.6 | 11.98 h | 69.9 min |

The ratio is a stable ~5–6×. METR's own limitations note is candid: CIs are
~2× in each direction, ~170 tasks, horizons differ **40–100× across domains**,
and the metric means *serial human labour replaceable at 50% success* — **not**
how long an agent runs unattended.

**Reliability decays much faster than mean success**: tau-bench GPT-4o retail
**pass^1 61.2% → pass^8 <25%**; τ²-bench telecom claude-3.7 **0.49 → 0.37 →
0.31 → 0.25** across k=1..4.

📄 **Governance decay** is the finding that validates a decision mecha already
made: with full context, policy violations **0%**; **after compaction, 30%
average and up to 59%**. When constraints survive summarization: 0%; when
dropped: 38%. The mitigation — *quarantining governance constraints from lossy
compaction* — restores 0%. **This is exactly mecha's invariant: taint lives on
`Conversation`, not in the messages, so compaction cannot launder it.**
⚠️ Unreplicated single-author preprint, but the mechanism is clear.

---

## Test-time scaling

📄 **Large Language Monkeys**: SWE-bench Lite **15.9% pass@1 → 56% pass@250**.
But the finding that matters is **the selection gap**: on MATH with
Llama-3-8B, *coverage* rises to ~95–98% at N=10,000 while selection methods
plateau by roughly N≈100. Repeated sampling converts to accuracy **only with
an automatic verifier**. ⚠️ The plateau's *height* (~38–40%) was read off a
figure by a summarizing fetch, not a table — **medium confidence**. The
plateau-at-~100-samples and the >55-point coverage gap are solid. The paper's
own abstract says "several hundred samples" where its body says ~100.

📄 **Inference Scaling fLaws** (Princeton) gives the theory and the constants:
an imperfect verifier's false-positive rate **cannot be reduced by resampling**
and imposes a compute-independent ceiling; **FPR *increases* with K**, and
weaker models false-positive more. Optimal **K ≤ 5** at a cost ratio of 4, and
**K = 0** once the ratio reaches 10. Empirically a learned selector recovers
**~50% of the random-to-oracle gap and no more** (CodeMonkeys: coverage 69.8%,
random 45.8%, **selected 57.4%**, at ~$4.58/instance and ~$2,300 total).

📄 **The verifier saturates, not the sample count** — R2E-Gym is the cleanest
evidence: a 32B model at 34.4% pass@1, where execution-based and execution-free
verifiers **each saturate at 42–43%** and only a *hybrid* reaches 51%. Adding
samples does not move a saturated verifier.

📄 And on the least-verifiable tasks, scaling verification **actively hurts** —
fraction of coverage gap recovered: Web-of-Lies 66.5%, AIME 57.1%, MATH 20.0%,
**Olympiad −11.2%**. Measured verifier error on MATH: **14% FPR / 17% FNR**,
and prompt-sensitive. Also worth knowing before spending on N:
**Consistency@50 equals Consistency@10,000 on AIME.**

**Tree search: measured gains, and the cost accounting is conspicuously
absent.** ✅ ToT (NeurIPS 2023) Game of 24: CoT 4.0% → **74%** — but its own
appendix prices that at $0.74/case against $0.47 for CoT best-of-100, the
authors note **"5–100× more generated tokens than CoT"**, and they recommend
ToT only where CoT struggles (GSM8K moves 86 → 90). LATS reports HumanEval
92.7% with **no retrievable cost accounting at all**, which for an MCTS-over-
trajectories method is itself the finding. Neither SWE-agent nor OpenHands uses
tree search.

⚠️ **Removed from an earlier draft of this file after a second verification
pass could not reach the source**: specific SWE-Search figures (25.7 → 31.0 at
14.1× cost, and an Appendix J compute-matched collapse). Also deliberately
*not* included, for the same reason: ProcessBench's PRM-generalization numbers.
The *direction* of that claim survives via a different source — search-based
test-time scaling works with some PRMs and **fails** with others on
out-of-distribution policies, with a measured ~2× length bias — but the
numbers should not be cited.

📰 Anthropic is the only lab publishing agentic best-of-N for SWE-bench
Verified, with a stable **+6.6 to +7.5pp** — method: parallel attempts,
**discard patches that break visible regression tests**, then internal scoring.
N is never disclosed.

---

## What this means for mecha

**The good news is that mecha's strongest verifier is already the right
shape.** `expect.verify` satisfies three of the four criteria: it *executes* a
command rather than judging, in a *staged private workspace*, and it grades an
exit code, which is a cheap no. Its tamper resistance is that the command is
**owner-authored in the case file**, out of the model's reach, and
`EvalCase::validate` refuses a `verify` case that is not sandboxed — so it
cannot assert against the shared fixture.

⚠️ **Corrected 2026-09-18.** An earlier draft of this paragraph claimed
`expect.verify` "hashes the test file first". It does not, and nothing else
here does either: `grep -rln "Sha256\|sha2::" mecha-core/src` returns no files.
The command is out of reach, but whatever it *asserts against* lives in the
model's own workspace, so a case whose verify command runs tests the model can
edit is tamper-*exposed* in exactly the way METR measured o3 defeating on 30.4%
of RE-Bench runs. `mismatch.rs` is the module that actually holds the line —
gold stays in the run record, outside the workspace. Treat that as the pattern
to copy, not this one. The fixture generator's "each kata must fail as
shipped *and* be solvable by a reference fix" is the same discipline applied to
the benchmark itself — which is the exact defect UTBoost found in 40.9% of
SWE-bench Lite entries.

**The judge-based checks are weaker than they look.** `expect.judge` and
`validate`'s followup probes are the AUROC 0.54–0.65 regime for detecting
whether something silently failed. The handoff already says "treat a single
judge failure as a prompt to read the answer, not as a result" — the research
says that instinct was right and probably understated. By contrast, the steer
and denial probes are **trace-graded**, and the proposals gate runs on those:
that is the deterministic half and it is worth more than the judged half.

**Implications, ordered:**

1. **Any in-run verification must be execution-grounded.** A critic step that
   asks the model to check its own work is measured at ~0 and sometimes
   negative. Post-conditions that *run something* (does it compile, does the
   test pass, did the file change) are the only kind worth adding.
2. **Never gate completion on self-report or an LLM judge.** If a ralph-style
   loop is built, its convergence test must be a command's exit code.
3. **A loop must evict its own failures, not summarize them.** Self-conditioning
   is 20–30pp at turn 100; fresh context per iteration is the measured fix, and
   it is the part of Ralph that actually works.
4. **Prefer adversarial or trace-graded checks over a single stronger critic** —
   a lone consultant makes the judge *worse* as it gets more persuasive.
5. **Periodic plan re-injection (~every 5 steps)** is the one replicated
   positive planning result and is cheap to try.
6. **pass^k, not pass@1**, for anything claiming reliability.
7. Taint-survives-compaction is externally validated; do not weaken it.

---

## Second pass, 2026-09-18: what the other harnesses ship

Prompted by "other harnesses have a verifier — what are they doing?". The
first pass surveyed the literature; this one surveys the products, and the
two answers do not conflict once the question the verifier is asked is
separated from the verifier itself.

### The survey

📰 **Claude Code** has five hook *handler* types — `command`, `http`,
`mcp_tool`, `prompt` (single-turn model evaluation), `agent` (a subagent with
Read/Grep/Glob, marked experimental) — across 30-odd lifecycle events. The
one that matters here is **`Stop`**: it fires when the model finishes
responding, receives the last assistant message and the transcript path, and
**exit code 2 prevents the turn from ending**. `PostToolUse` explicitly
cannot block, because the tool already ran.

📰 **Claude Science** is the actor–critic: a reviewer agent *"inspects the
outputs, flagging incorrect citations, untraceable numbers, and figures that
don't match their underlying code."* The page is explicit that the emphasis
is **reproducibility rather than re-execution** — every figure carries *"the
exact code and environment that produced it, a plain-language description of
how it was created, and the full message history."*

📰 **Long-running Claude** (the Boltzmann solver) is the opposite pole and
agrees with the first pass exactly: an external **test oracle** (CLASS C as
reference implementation), a `CHANGELOG.md` recording failed approaches and
why, and a Ralph loop whose exit condition is a *number* — "until the success
criterion of 0.1% accuracy across the entire parameter range is achieved".

🔮 **Codex** is the light version, and the 2026 community consensus states
the rule the first pass derived: self-checking is *"the first line, not the
final verdict."*

### The distinction that reconciles the survey with the literature

**A critic is refuted as a judge of quality and sound as a resolver of
references.** The AUROC 0.54–0.65 result is for the open question *"did this
succeed?"*, where the judge's own competence bounds the answer. Claude
Science's reviewer is never asked that. It is asked whether a number appears
in the source it cites and whether a figure matches the code that claims to
have produced it — questions with a **referent**, whose failure mode is a
missing referent rather than a bad opinion.

**The provenance is the mechanism; the reviewer only walks it.** Attaching
code, environment and message history to every artifact is what converts the
unanswerable question into the checkable one. Note where that lands for us:
CLAUDE.md's *Reporting rules* — never state a number you did not measure —
is the same property, enforced by prompting rather than by structure, and the
three miscounts of 2026-09-05 are what prompting costs.

### Against a lone adversarial verifier

**Adversarial framing is safe as a debate and unsafe as a single critic**,
and the first pass already contains both halves. ✅ Khan: two advocates 76%,
one consultant 54%, naive 48% — and as the *single* consultant's
persuasiveness rises, judge accuracy **falls**. 📄 Stechly is the specific
refutation of an adversary on a completion gate: "evil feedback" — telling
GPT-4 that a *correct* edge is wrong — produces a **94% "fix" rate,
identical to real first-error feedback.** The model edits whatever it is
pointed at without discriminating, so a critic tuned to point at more things
buys false positives at full price. 📄 And an imperfect verifier's FPR is a
compute-independent ceiling: resampling does not lower it.

So: **no judgement-based adversary on a completion gate.** Two structures
survive — an adversarial *pair* whose independence is structural, and an
adversary whose claims are *resolvable against a referent*.

### What is already built, and where it stops

- `step::CheckRequest` is the frozen declared check: the plan step names a
  tool and input, the **loop** dispatches it, and `MAX_CHECKS_PER_RUN` is 16.
  The forge direction is the safe one — the module's own note is that a model
  emitting `step.check` as a `tool_use` lands as `unknown`, which
  `Outcome::of` reads as `Failed`, so *"it can manufacture a failed check
  against itself, never a passing one."* `appraise` reads `CheckFailed`
  before the last-attempt readings, which is how a run whose final call
  succeeded while its claim did not still gets caught. **Reachable only from
  `todo.rs`** — a run that never plans cannot declare a check.
- `anticipation::assess` is prospective regret, and deliberately **not a
  minimisation**: a pure function from `Evidence` to a *set* of concerns plus
  one of four `Response`s, with `uncertainty` as a bool because "no
  probabilistic confidence is invented from an unverified claim". `Kind::Regret`
  is pushed only when `unverified && affordable == Some(true)` — regret means
  *there was a named check you could afford and did not run*, which is an
  unexercised option rather than a feeling. `outbox.rs` re-derives the
  assessment and asserts equality rather than trusting the stored label.
  **Wired to the send path only**; steps do not populate `Evidence`.
- `gossip::reader` is the sound adversarial pair, built for the graph:
  commit-then-reveal enforced structurally, one `LensedSearch` tool per child
  with its sources nailed shut and removed from the schema, so *"a child
  cannot widen its own lens to see what its partner sees, which would collapse
  the two witnesses into one."* The design note names why this is Rust and not
  a prompt: a parent told not to show B what A said *can simply not comply,
  and nothing would notice.*
- `hooks::Event` has exactly three variants — `PreTool`, `PostTool`,
  `SessionEnd`. Only `PreTool` can deny. `session_end` runs after the fact and
  logs a warning on failure. **There is no gate that can refuse to let a run
  end**, which is the `Stop` capability every surveyed harness has and we do
  not.

### Implications, ordered

8. **A blocking end-of-run gate is the missing mechanism**, and implication 2
   already depends on one: a convergence test that is a command's exit code
   needs somewhere to run and something to refuse. `session_end` is the wrong
   event because it cannot say no.
9. **Make the reporting rule structural before making it adversarial.** A
   tool result that carried its command and exit status as fields would let a
   non-model checker flag figures in a final message that trace to nothing —
   the *untraceable numbers* check, with no judge in it.
10. **If an adversarial verifier is built, build the pair, not the critic** —
    `gossip`'s lens is the pattern, and the task-completion analogue of
    independent sources is independent *evidence channels* (one reader sees
    only the transcript, the other only the workspace diff).
11. **Widen `CheckRequest` past `todo.rs`** and let step boundaries populate
    `anticipation::Evidence`, so `Response::Verify` fires where the check
    machinery can act on it. Both halves exist and do not meet.

### Scope, 2026-09-18: the grounding primitive is hand-rolled four times

Surveyed by reading every `parse_*` boundary in `mecha-core/src` and every
caller of `replay::extract`. **The same primitive — build the set of what
the run actually received, then test a model's claim against it by literal
containment — exists in four places, in two polarities, and none of them
shares a line.**

| Instance | Polarity | Source set | Referent | Compaction-aware |
|---|---|---|---|---|
| `gossip::evidence_from` + `grounded_claims` | admit if it cites | `replay::extract`, `kg_search` results only | episode id + quote ≥ 12 chars, literal substring | **no** |
| `outbox_source::from_messages` | join | raw `Block::ToolResult` walk | `tool_use_id` / provider id | **yes** — first seen wins |
| `diagnose::carries_over` | *reject* if it quotes | what the diagnostician read | 8-word window | n/a |
| `mismatch` criterion pointers | resolve | owner record | JSON pointer | n/a |

**The general module must inherit `outbox_source`'s compaction rule, and
`gossip` does not have it.** `evict_superseded_results` rewrites a result's
content in place under the same `tool_use_id`, so one id maps to two
contents: the thing the model read, and `[superseded: …]`. `outbox_source`
takes the first seen for that reason; `replay::extract` has no such rule
(no `superseded`, `or_insert` or `messages_ever` in the file). Harmless in
gossip today because its conversations are fresh and short; a wrong-evidence
bug the day the walk is pointed at a long run.

**Where the primitive is missing and a prompt does the job instead:**

- `frontdoor::Extraction::dates_mentioned` — the prompt says *"as written in
  the text"* and the schema requires the field; `parse_extraction` only
  deserialises. Nothing checks a date is a substring of the submitted prose.
- `mail_triage::Verdict::deadline` — format-checked (`YYYY-MM-DD`) and never
  checked against the thread. A normalised date will not appear literally, so
  grounding it needs a `quote` span on the schema first.
- `compact::parse_omissions` — a judged check on free prose; the hard case,
  noted and not scoped.
- `distill::Correction::wrong` — a claim about the session, unchecked; the
  graph's review queue is the downstream guardrail, so lowest.

**Shape.** A `grounding` module with `Evidence { id, source, text }`
(`occurred_at` is graph-specific and stays with the caller), a transcript
walk that takes the caller's packet parser and owns first-seen-wins and
error exclusion, an `admit` that partitions claims into grounded and
ungrounded, and `carries_over` moved across with its window as a parameter.
Not a tool: nothing enters the `Registry`. Each caller's thresholds (12
chars, 8 words, 25 items, 4000 chars) stay per-caller, because each was set
against its own failure and none was measured for the others.

**What stays put.** Gossip's `CLAIM | EPISODE | QUOTE` line format and lens
filter are its wire format. `outbox_source`'s `Join::Asked` ranking and
`MIN_RETURNED_ID_CHARS` are a join, not an admission. The exchange itself is
untouched — its open namesake defect (above) is the reason.

**Order.** (1) module + gossip as the first caller, a pure refactor with
`cargo test -- --list` diffed to zero; (2) `frontdoor` dates as the first new
caller, a *finding* on the record rather than a block; (3) `outbox_source`
consumes the shared walk, which is how the compaction rule lands in the
module; (4) the triage `quote` span. Invariants go in `ARCHITECTURE.md`'s
own section, per CLAUDE.md.

---

## Citation traps found while researching

Two **fake arXiv-mimicking domains** served plausible papers with fabricated
statistics and ranked well in search: `clawrxiv.io` and `centaurxiv.org`.
Anything cited from a domain ending in `xiv` that is not `arxiv.org` must be
verified against arXiv directly.

Two systemic issues in this literature: **single-run reporting with no variance
is the norm** (one study measures 5–15pp of pass@1 standard deviation across
seeds on AIME'24, and finds most published RL reasoning gains are not
statistically significant under standardized re-evaluation), and
**compute-matched controls are almost always missing** — wherever they have
been added, much of the claimed planning or critique benefit turns out to be
the benefit of spending more tokens.
