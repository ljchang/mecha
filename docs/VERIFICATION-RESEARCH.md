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
Llama-3-8B, *coverage* rises 82.9% @ N=100 → ~95–98% @ N=10,000 while majority
voting and reward-model selection **plateau at 38–40% by N≈100**. Repeated
sampling converts to accuracy **only with an automatic verifier**.

📄 **Inference Scaling fLaws** (Princeton) gives the theory and a constant: an
imperfect verifier's false-positive rate **cannot be reduced by resampling** and
imposes a compute-independent ceiling; FPR *increases* with K. *"Optimal
sampling attempts are often fewer than 10."* Empirically, across six
independent systems a learned selector recovers **~50% of the random-to-oracle
gap and no more** (CodeMonkeys 45.8 → 57.4 against a 69.8 oracle; SWE-Gym 51%;
Trae 55%).

📄 And on the least-verifiable tasks, scaling verification **actively hurts** —
fraction of coverage gap recovered: Web-of-Lies 66.5%, AIME 57.1%, MATH 20.0%,
**Olympiad −11.2%**.

**Tree search is measured but collapses under a fair control.** ✅ SWE-Search
25.7% → 31.0% at **14.1× the cost** (~$33 per additional resolved issue) — and
its own Appendix J shows that **compute-matched against plain repeated
sampling the +23% headline becomes +0.6 to +5.3 points**, with plain resampling
*beating* single-shot SWE-Search on two of four models. Neither SWE-agent nor
OpenHands uses tree search.

📰 Anthropic is the only lab publishing agentic best-of-N for SWE-bench
Verified, with a stable **+6.6 to +7.5pp** — method: parallel attempts,
**discard patches that break visible regression tests**, then internal scoring.
N is never disclosed.

---

## What this means for mecha

**The good news is that mecha's strongest verifier is already the right
shape.** `expect.verify` satisfies all four criteria: it *executes* a command
rather than judging, in a *staged private workspace* the run does not control,
and it **hashes the test file first** so a model that edits tests until they
pass fails. That last property is precisely the tamper resistance METR measured
o3 defeating 30.4% of the time. The fixture generator's "each kata must fail as
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
