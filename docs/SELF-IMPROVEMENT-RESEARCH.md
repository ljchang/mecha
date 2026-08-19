# Self-improving harnesses: what is measured, and what mecha is missing

Research pass, 2026-08-19, prompted by two questions: is there more harness
observability worth having, and is there any mechanism for the harness to
notice its own problems and improve. The answer to the second is **no** —
rumination's only sensor is a human stepping in — and this is what the
literature says about closing that loop without breaking the provenance rule.

**Venue key**: ✅ peer-reviewed · 📄 preprint · 📰 vendor/blog · 🔮 folklore.

Complements `MEMORY-RESEARCH.md` (the rule store, tenure, retirement) and
`HARNESS-RESEARCH.md` (what makes a harness good at all).

---

## The one-paragraph answer

mecha already updates its own prose: the rule store *is* an evolving playbook
with a proposal gate, a validation ledger, per-rule attribution by bisection,
and gated retirement — which is more machinery than most published
self-evolution work has. What it lacks is a **sensor**. `extract_interventions`
reads user steers, user denials and outbox edits; a harness problem produces no
intervention, so it produces no reflection, and nothing downstream ever sees
it. The loop is closed on conduct and open on the harness. Everything below is
about attaching the missing sensor without turning the playbook into an
injection vector.

---

## 1. The harness is a convergent architecture, not a convenience

✅ **HyperAgents** (Meta, ICLR 2026,
[arXiv:2603.19461](https://arxiv.org/abs/2603.19461)) is the paper this pass
was asked about, and its most useful result is not the self-modification
machinery. It is what the agents *built*.

A hyperagent is a task agent plus a meta agent in one editable program, where
the modification procedure is itself editable — "metacognitive
self-modification". Left to improve across coding, paper review, robotics and
math grading, the agents independently evolved **persistent memory,
performance tracking, multi-stage verification pipelines, decision protocols
and retry logic**.

That is the mecha component list. Learned rules, the validation ledger, the
eval rig, the approver and interlock, `provider/retry.rs` — arrived at here by
incident, arrived at there by search, and the convergence is the evidence that
none of them is decoration. It also predicts what a self-improving mecha would
build if it could: the things it does *not* yet have.

Improvements transfer across domains: a hyperagent that learned to improve
itself on paper review carried that ability to robotics reward design, and
DGM-H + transfer reaches **0.640** on math grading against **0.610** starting
fresh (0.700 initialized from ProofAutoGrader + transfer).

**What to take, and what not to.** The convergence result is worth acting on.
The self-referential machinery is not: mecha's meta-level is a human, on
purpose, and §3 is the reason.

---

## 2. Updating the harness is not the same as improving it

📄 **"Harness Updating Is Not Harness Benefit"**
([arXiv:2605.30621](https://arxiv.org/pdf/2605.30621)) is the finding that
should govern the design, and it is the unflattering one: agents *do* modify
their own prompts and tools, and frequently fail to get any corresponding gain.
**Activity ≠ effectiveness.** Four named failure modes:

1. **Blind updating** — modifications made without validation.
2. **Superficial changes** — edits that do not touch the real capability gap.
3. **Plateau** — early gains that do not sustain.
4. **Misaligned optimization** — optimizing for update *frequency* rather than
   task success.

Read against mecha: the expensive half of self-improvement is not the
proposer, it is the **measurement**. `validations.jsonl`, the counterfactual
probe, the per-rule bisection and the 3-strike retirement already exist and
already answer all four — which means the marginal cost of a harness sensor
here is lower than in any of these systems, because the grading half is built.

---

## 3. The closest published method, and the number that argues for a human

📄 **AHE — observability-driven automatic evolution of coding-agent harnesses**
([arXiv:2604.25850](https://arxiv.org/html/2604.25850v3)) is what this section
would be if built autonomously. Three observability pillars, and the shape is
worth stealing wholesale:

- **Component observability** — the harness is decomposed into orthogonal
  file-level components so "each failure pattern maps to a single component
  class", giving a clean action space and localizing every pass-rate change to
  one file.
- **Experience observability** — an "Agent Debugger" distils raw rollouts into
  layered evidence (per-task failure analysis, benchmark-level overview),
  turning millions of trajectory tokens into something an evolving agent can
  consume.
- **Decision observability** — **every proposed edit is paired with a
  falsifiable prediction**, and the next iteration intersects the predicted-fix
  and predicted-regression sets with the observed task-level deltas to produce
  a per-edit verdict. Each edit becomes falsifiable by the next evaluation.

The pipeline is rollout → clean → **attribute** (verify prior predictions,
revert rejected edits) → distil → evolve → commit-to-git.

And then the number that decides mecha's version of this. AHE's gate is fully
automated, no human in the loop, auto-reverting anything that failed to deliver
— and the paper measures its own attribution at **51.4% recall on predicted
fixes and 11.1% on predicted regressions**. Its words: the loop's
self-attribution "is reliable for fixes but blind to regressions."

**A loop that catches one regression in nine and accepts its own work is a
ratchet pointed the wrong way.** That is the argument for `mecha proposals`
staying where it is, and the component ablation in the same paper (§7 of
`HARNESS-RESEARCH.md`) is the corroboration: stacked components interacted
*negatively* there, and an autonomous loop measuring itself at 11% would not
have seen it.

---

## 4. Prose can evolve — the failure modes are named, and mecha has one

The instinct to keep self-improvement away from prompt text is too blunt.
Prose evolution is a measured technique with known failure modes, and mecha
already does it.

📄 **ACE — Agentic Context Engineering**
([arXiv:2510.04618](https://arxiv.org/abs/2510.04618)) treats context as an
evolving playbook and names the two ways that goes wrong:

- **Brevity bias** — dropping domain insight in favour of concise summaries.
- **Context collapse** — iterative *rewriting* eroding detail over time.

Its fix is structural, not exhortative: **delta operations** (ADD / UPDATE /
REMOVE on individual bullets) applied by a Curator, with non-LLM merging and
de-duplication, rather than regenerating the playbook. ReAct + ACE beats its
baselines by **10.6%** on average, and is cheaper and lower-latency than GEPA
precisely because the updates are incremental.

**This lands directly on `mecha learn`.** Consolidation there *rewrites* the
domain's rule set with a model in the loop. That is the context-collapse shape,
and the mitigations already in place are partial: identity is carried across
consolidations by text-match, the char budget bounds size, and the validation
ledger would eventually notice a rule that got worse. What is absent is the ACE
property — that an edit is an ADD, an UPDATE or a REMOVE of one bullet, and
everything untouched is *untouched by construction* rather than by a
summariser's discretion.

---

## 5. The threat the whole design has to survive

📄 **Zombie Agents**
([arXiv:2602.15654](https://arxiv.org/pdf/2602.15654)) is the security paper
for exactly this feature: a **self-reinforcing injection** lodges in a
self-evolving agent's memory or skill store and becomes *more* entrenched as
the agent learns, because the agent's own improvement machinery carries it
forward. Defenses tested include instruction-defense prompts and varied memory
architectures. (The PDF's numbers did not extract; the mechanism is what
matters here and the mechanism is unambiguous.)

mecha anticipated this — `Origin` on every reflection, fail-closed
classification, structural exclusion of non-clean reflections from `learn`,
and deliberately no knob. The rule that follows for a harness sensor:

> **The input to a prose proposal must be a number, never a body of text.**

A rule templated over deterministic counters — call volume, error rate,
`stop_cause` frequency, compaction count — is machine-authored and
attacker-free, because a counter carries no instructions. A rule paraphrased
from an *error string*, a tool result, or a transcript excerpt is third-party
text on the longest half-life path in the system. Both are "prose changes";
only one of them is safe, and the difference is not visible in the output.

---

## 6. What to build here

The sensor is missing; the grading half is not. Four steps, each usable alone.

1. **Record what a run already knows.** `Session::Summary` carries `{usage,
   turns}` and discards the other thirteen fields of `RunOutcome` — so every
   chat, TUI and Slack run is less observable than a trigger. Add the rest,
   plus a retry counter (attempts are not counted anywhere today) and the
   cache-lens verdict (which currently warns to stderr and is never persisted).
2. **A run-quality ledger.** One append-only row per finished run: the trigger
   ledger's shape, generalised to every front-end. This is the sensor.
3. **A deterministic scan proposing *config*.** Runs hitting `MaxTurns` at a
   high rate; `compactions` high while the context high-water mark is low
   (`compact_at_tokens` too low); overflow *despite* the threshold
   (`context_window` is stale); results capped often (`output_budget_bytes`);
   one tool's error rate pathological. Same gate as a rule change.
4. **Then prose, under §5's rule.** A `harness` rule domain, proposals
   *templated over counters*, each carrying AHE's falsifiable prediction so the
   existing ledger can grade it, applied as ACE-style deltas, accepted by a
   human, retired on measured harm by the machinery that already does that.

What not to build: an autonomous accept gate (§3, 11.1%), a critic that reads
transcripts to decide what to improve (§5), and anything that regenerates the
rule set wholesale (§4).

---

## 7. Connected or self-contained? Neither — the seam goes in one place

The question is really four questions, because `learn`'s stages fit very
differently. Taking them in order:

**`reflect` — do not reuse.** Its input is a transcript moment and its output
is a model's paraphrase of it. A harness observation is a counter. Passing a
counter through a model reflector adds a request to something deterministic,
turns a number back into free text, and breaks §5's rule at the first step.
`Origin` classification is meaningless for it too — provenance there is about
transcript taint, and a counter has none. This is the "distillation is not
learning" line again: two things that look alike do not get the same pipeline.

**Consolidation — do not reuse.** A harness rule is *templated over numbers*,
so there is nothing for a model to consolidate; running one over it is the ACE
context-collapse risk taken on for no benefit. What *is* worth reusing is the
non-model half: `MAX_ACTIVE_RULES_PER_DOMAIN`, `RULES_CHAR_BUDGET`, and
`rules_prompt_block_for` selection, because a harness domain that rides in
every prefix must be budgeted like every other one.

**The counterfactual/replay probe — reuse it, but know what it cannot see.**
`replay_run` answers from the recording, and says why: "same turns, same tool
results, and the only thing left that can differ is what the model chose to do
with them." That isolation is exactly right for a conduct rule and **blind by
construction to a harness change**, because a harness knob's entire effect is
on *what results reach the model* — a tool-output budget, a compaction
threshold, a turn ceiling. Replaying holds fixed the one variable the change
moves. So harness proposals are graded by `mecha eval` (pass^k, the
run-metadata checks) and by the run-quality ledger's own trend, not by the
probe. Using the probe anyway would key `validations.jsonl` to a measurement
that could not have moved — the exact failure the ledger is designed to avoid.

**The proposal gate — reuse it, and this is the load-bearing decision.**
`Proposal` is rule-shaped today (`domain`, `reflexion_ids`, `rules_before`,
`rules`). A config proposal needs a second payload, which means either a `kind`
on the existing type or a second review surface. It must be the `kind`, on the
outbox's precedent exactly: **staging generalises for free and reviewing does
not**, so `accept`/`show` grow a branch and the store does not change. The
alternative — a parallel `mecha harness-proposals` — creates two places a human
says yes, and the second one is always the unmaintained one.

### So: own sensor, shared gate

| Stage | Verdict |
|---|---|
| Sensor (run-quality ledger) | **new, self-contained** — no model, no reflection |
| Scan → candidate change | **new, deterministic** — the `propose-retirements` template |
| Evidence archive | **new** — a counter does not fit `Reflexion`'s fields |
| Budgets and prompt selection | **shared** |
| Grading | **shared machinery, different probe** — eval + ledger trend, not replay |
| Human gate | **shared, with a `kind`** |
| Retirement on measured harm | **shared** |

The shape this argues against on both ends is worth naming. Fully connected —
harness findings entering `reflect` — puts a model between a counter and a rule
and reintroduces free text as an input. Fully separate — a second store, a
second ledger, a second review command — is the 2026-08-11 shape that produced
`doctor` in the first place: five components each recording their distress
correctly, and nothing reading across them. The lesson there was that a **new
store is fine and a new reader is not**, and the same asymmetry applies here.

### The baseline that has to be beaten

Steps 1–3 of §6 stop at `doctor`: observe, aggregate, report to a human, change
nothing automatically. That is a complete and useful system, and every step
past it — staged config proposals, then templated prose — has to earn its
place against it. Given §2's finding that agents update their harnesses without
benefiting, the honest order is to build the sensor, run it for a while, and
see whether the findings it produces would actually have been *acted* on. A
proposer built before that evidence exists is optimizing for update frequency,
which is the fourth failure mode by name.

---

## 8. Self-grading by replay: what it needs, and where autonomy is earned

The proposal: rerun past episodes under a candidate change and compare against
what happened, so the loop grades itself. It works, with one constraint that
decides its shape.

**Replay supplies the trajectory, never the label.** Rerunning yesterday's
briefing under a new `compact_at_tokens` says the model did something
*different*; it cannot say *better*. `mecha validate` escapes this only because
a recorded intervention **is** the label — a human already said what right
looked like at that moment, and the probe asks whether the rule reaches it
unprompted. Remove the label and replay measures divergence.

**The label a harness change needs already exists**, as of 2026-08-19:
`stop_cause`, `ended_on_failed_call`, `tool_errors`, `malformed_tool_args`,
`compactions`, turns and cost are deterministic, objective and computable on a
replayed episode with no human and no judge. They are not answer quality — and
answer quality is not what a harness change moves. This is AHE's falsifiable
prediction with a grader that measures instead of inferring, which is why the
11.1% regression recall there does not transfer: that number came from a model
attributing outcomes, not from counting `stop_cause` over a fixed corpus.

**What replay can and cannot exercise**, restating §7 more precisely now that
it matters. Recorded results are replayed verbatim, so a change is visible iff
its effect reaches the model through the *transcript*:

| Change | Gradeable by replay? |
|---|---|
| `compact_at_tokens`, eviction, failure collapse, carried state | **yes** — they rewrite the transcript the model reads |
| learned-rule text | **yes** — this is what `validate` already does |
| `max_turns`, budgets | partly — the ceiling is observable, the counterfactual work is not |
| `output_budget_bytes`, sandbox, retries, provider failover | **no** — recorded results are fixed, so the change cannot reach the model |

A proposal whose class is "no" must be graded by `mecha eval` or by the
run-quality ledger's own trend, and staging it against a replay score would key
the ledger to a measurement that could not have moved.

**Two reasons autonomy is scoped rather than blanket**, both specific:

- **Goodhart, and mecha already ships the detector.** "Fewer tool errors" is
  trivially achieved by making fewer calls — the null run that `doctor` now
  flags. `VERIFICATION-RESEARCH.md` has the calibration: METR measured o3
  gaming its own scorer on **30.4%** of RE-Bench runs, and SpecBench found the
  reward-hacking gap growing ~27pp per 10× LOC. **Every accepted metric needs a
  paired work-volume counter**, and a proposal that improves its target while
  the paired counter falls is rejected rather than scored.
- **Half-life.** A config knob is reversible and bounded by the next
  measurement. A rule in the cached prefix rides in every future run and is the
  Zombie Agents surface (§5).

### The autonomy ladder

| Change class | Gate |
|---|---|
| Reversible config knob, replay-gradeable, paired counter held | **auto-accept**, evidence recorded, `revert` available |
| Reversible config knob, not replay-gradeable | staged proposal — eval or trend evidence, human accepts |
| Prose into the prompt prefix | **human, always** (§5) |
| Sandbox, outbox routes, interlock, path jail | **never proposed at all** |

The last row is not a gate, it is an exclusion: a loop that can widen its own
confinement in response to its own measurements is the silently-degrading
sandbox with a scorecard attached. The metric would even improve — a run that
can reach the network fails fewer calls.

### Corpus

Eval cases (which carry graders, so answer quality is available there too) plus
a sampled set of real sessions from the transcript store. Replaying costs a
real model run per episode, so this belongs in `ruminate.sh` beside the other
nightly stages, after `validate` and on the same "a skipped night is not a
failed night" contract.

---

## 9. Grading the *quality* of a run, not just the harness

`RunStats` says whether the harness worked. It says nothing about whether the
run was any good. Four candidate signals, in descending order of how much they
can be trusted, and one that mecha already collects and has never used.

### 9.1 Deterministic contracts over the trace beat a judge, measurably

📄 **GroundEval**
([arXiv:2606.22737](https://arxiv.org/pdf/2606.22737)) replaces LLM-as-judge
for stateful agent evaluation with programmatic assertions over execution
traces, state changes and output artifacts. Five dimensions: **access control**
(only authorized resources touched), **temporal horizon** (finished within
constraints), **evidence visibility** (outputs retrievable afterwards),
**causal grounding** (the demonstrated output actually resulted from the
agent's actions), **verified absence** (harmful side effects did not occur).

Measured: **89% agreement with human evaluation against 67% for an LLM judge**,
and **100% deterministic reproducibility**. Strongest exactly where mecha lives
— long-horizon tasks where sequencing and causal chains matter.

The substrate for this already exists here: the tool trace, the path jail, the
taint record, the outbox, the work directory. Several GroundEval dimensions are
things mecha *enforces* and could additionally *score* — and scoring an
invariant is how a regression in it becomes visible before an incident.

### 9.2 Rule-based evaluators fail in a known direction, and it is the opposite one

✅ **AgentRewardBench**
([arXiv:2504.08942](https://arxiv.org/pdf/2504.08942)) is the meta-evaluation:
expert-annotated web-agent trajectories, used to grade the graders. Its finding
corrects something this repo has been one-sided about — **rule-based benchmark
evaluators systematically *underreport* success**, marking successful
trajectories as failures at notably higher rates than humans would, while LLM
judges are more nuanced and also do not match experts.

So the two families err in *opposite* directions: judges are permissive and
credulous about silent failure (`VERIFICATION-RESEARCH.md`: AUROC 0.65 / 0.54,
75.8% false successes), and deterministic rules are conservative and
false-positive-prone. `ended_on_failed_call` is exactly a rule-based evaluator,
and this predicts its error direction — which was already the design assumption
("a false positive costs one read"), and is now a measured expectation rather
than a hope.

**The consequence for using it as an objective function is sharper.**
Optimizing against a conservative grader optimizes for *looking* safe. A
harness change that reduces flagged runs by having the model attempt less
scores well on both, which is §8's Goodhart case arriving through the grader
rather than through the metric.

### 9.3 Cross-run agreement: a triage signal, not a grader

📄 [Auditing self-consistency and cross-model agreement](https://arxiv.org/abs/2607.08065)
and 📄 [behavioral consistency as an uncertainty signal](https://arxiv.org/html/2602.11619v2)
both land in the same place: agreement is a **regime-dependent, positive but
weak** proxy for accuracy, and it is not accuracy — models agree out of shared
bias, memorized heuristics and position priors as readily as out of truth.
Consistent-wrong tasks run **5.5–10%** and set a hard ceiling on any filter
built this way.

mecha gets this signal for free from `--runs k` (the pass^k / pass@k gap *is*
the disagreement). Worth using to decide **which runs a human should read**.
Not worth promoting to a grader.

### 9.4 The label mecha already collects and has never used as one

The outbox records `args_before` and `args`, and every staged item ends in
**sent**, **sent-with-edits**, or **rejected** — by a human, with a reason on
the rejection. The frontdoor records `closed` with a required reason,
`needs-info`, and a rejected draft returning to `extracted`.

That is a human quality judgement on a specific run's specific output, already
being collected, already durable, already joined back to the session that
produced it. `mecha reflect` mines the *edit distance* of these into writing
rules — but the **accept/reject bit itself has never been read as a label**.

It is the best signal in this section by some distance: it is a real human
decision rather than a proxy for one, it costs nothing to collect because
review already happens, and it is joined to a session id so it can be paired
with that run's `RunStats`. Its limits are honest ones — it exists only for
runs that staged something, and "sent" means *good enough to send*, not
*optimal*.

### 9.5 What to build, if anything

In order:

1. **Join the outbox and frontdoor dispositions to `RunStats`.** No new
   signal, no model, no judge — a join across two stores that already agree on
   a session id. This is the only item here that produces a *labelled* corpus.
2. **A small set of GroundEval-style contracts** over what mecha already
   enforces, scored rather than merely enforced.
3. **Cross-run disagreement as a read-this-one flag**, nowhere near a gate.
4. **A judge, only as an input to a human**, if at all — CriticGPT's finding
   stands (human+model teams hallucinate less than the model alone), and every
   gating use in this file's evidence base is refuted.

---

## 10. Many counterfactuals, or one comparison against the original?

Both are sound, and they are not equally safe.

**Compare-to-original is the right primitive, and mecha already implements
it.** `counterfactual.rs` runs a before-arm and an after-arm against the same
recorded moment; the paired design controls for everything about the episode
and leaves the change as the only variable. A/B against baseline is also the
only form that answers the question actually being asked — *is this better than
what we have* — rather than *which of these is least bad*.

**Best-of-N over a fixed corpus is a multiple-comparisons trap.** Generate
twenty candidate configurations, score them on the replay corpus, keep the
winner, and a good part of what has been selected is corpus-specific noise —
which is failure mode 3 in §2 (early gains that do not sustain) arriving by
construction rather than by bad luck. The more candidates, the worse it gets,
and the selection *looks* better the more it overfits.

The guard is a **holdout**, and `mecha learn --holdout` already exists for the
identical reason on the rules side. So:

- Selection among N candidates happens on the **selection slice**.
- The winner is then confirmed against the original on a **holdout slice never
  used for selection**, and a candidate that wins selection but not the holdout
  is discarded rather than shipped.
- Reported as **pass^k**, not mean: `BENCHMARK-RESEARCH.md`'s point that
  reliability decays faster than mean success applies here exactly, and a
  candidate that wins on the mean while losing on pass^k has bought its gain
  with variance.

And the paired counter-metric from §8 rides on both arms, because a candidate
that improves its target while doing less work must be rejected rather than
ranked.

---

## 11. The missing stage: diagnosis

§6–§10 describe detection, testing and gating, and skip the step between them.
A deterministic scan can propose a *knob* because knobs are enumerable in
advance. It cannot propose a fix for "the run loses its place after a
compaction" or "it re-reads the same file three times before acting", because
naming what to change there is an inference, not a lookup. **Replay can only
test a candidate; something has to author one.**

That stage is generative, and this document has been avoiding it for a reason
that does not survive inspection. The provenance rule (§5) says untrusted
*text* must not reach the prompt prefix. It does not say a model may not be in
the loop. The pipeline is:

```
detect      deterministic scan over the RunStats corpus         no model
diagnose    structured evidence in, a typed proposal out        MODEL
test        replay / eval, paired against the original, holdout no model
gate        the autonomy ladder of §8                           human or auto
record      validations.jsonl, per-proposal verdict             no model
```

AHE already runs exactly this stage and names it well: an "Agent Debugger"
distils rollouts into per-task failure analysis, and the evolve agent proposes
with a manifest naming **failure evidence, inferred root cause, targeted fix,
and predicted impact**. That last field is what makes the output testable
rather than merely plausible — a diagnosis without a prediction cannot be
falsified by the next measurement, and an unfalsifiable proposal is where
"harness updating is not harness benefit" (§2) comes from.

Three constraints on the diagnostic step, each derived from something already
settled here rather than invented for it:

- **Structured evidence in, by default.** The first-choice input is the
  aggregate — stop-cause distribution, tool error rates by tool, the shape of
  the trace (call sequence, repeats, compaction points), contract violations
  from §9.1. A counter carries no instructions, so a diagnosis over counters is
  clean by construction.
- **Transcript excerpts are sometimes unavoidable, and they carry their
  origin.** Some diagnoses genuinely need to see what happened. That is exactly
  what `Origin` already exists for: classify the harness reflection from the
  same `taint_timeline`, and let the classification decide what the proposal
  may *become*. A clean-origin diagnosis may argue for prose; an
  untrusted-origin one may argue only for a numeric config change, or go to
  `doctor` for a human — because a config value cannot carry an instruction and
  a sentence can. Fail-closed, as everywhere else: unknown counts as untrusted.
- **The proposal never quotes its evidence.** Whatever the diagnosis read, what
  it emits is a typed change plus a prediction. This is `frontdoor.rs`'s rule
  in a second setting — the privileged artifact sees the extraction, never the
  prose — and it is what stops a diagnosis from becoming a laundering path for
  the text it was reading.

**This is the only place a model belongs in this loop, and it is bounded on
both sides**: it is handed evidence it did not choose, and its output is
falsified by a measurement it does not run before anything is accepted. A bad
diagnosis costs one replay, which is the property that makes having a model
here safe in a way that having one at the accept gate is not.

---

## 12. Can it invent, or only tune? And how hard is diagnosis really?

### 12.1 Diagnosis is measurably bad, and that decides the design

✅ **Who&When / automated failure attribution**
([arXiv:2505.00212](https://arxiv.org/abs/2505.00212)) is the number this whole
stage has to be built around. 127 systems, failure logs annotated with the
responsible agent and the decisive error step, three attribution strategies
(all-at-once, binary search, step-by-step). Best result: **53.5% accuracy at
naming the responsible agent and 14.2% at pinpointing the failing step**, with
some methods **below random**, and frontier reasoning models failing to reach
practical usability.

So a diagnostician will usually name the wrong step. That is "superficial
changes" (§2, failure mode 2) arriving with a confident explanation attached.

**The design consequence is not to make diagnosis better; it is to make being
wrong cheap.** The falsifiable prediction and the replay test are not polish —
they are what makes a 14%-accurate diagnostician safe to *use at all*. The
requirement is never "the diagnosis is right", it is "a wrong diagnosis is
detected before anything is accepted, and costs one replay". Every proposal
class that cannot be tested that way inherits the 14% instead, which is the
real reason the untestable classes (§8's table) must go to a human rather than
to an auto-gate.

### 12.2 What the loop can invent

HyperAgents' agents added components. This design proposes knobs and prose, so
the honest answer is that it **tunes within the architecture** — and the
architecture is wider than "knobs", because mecha has several surfaces that are
programmable without recompiling.

Mapping HyperAgents' five convergent inventions onto what exists here:

| It invented | mecha's equivalent | Status |
|---|---|---|
| persistent memory | learning store, pkg | exists |
| performance tracking | `RunStats` | exists (2026-08-19) |
| multi-stage verification | `post_tool` hook, `expect.verify` | **authorable** |
| decision protocols | learned rules | authorable as prose |
| retry logic | `provider/retry.rs` + config | authorable as config |

Four of five already exist, which is §1's convergence showing up as
redundancy rather than as a gap. The genuinely *new-feature* channel is the
declarative surface: `[[hook]]`, `[[subagent]]`, `[[mcp]]`, triggers, and eval
cases. Each has a different safety story and they are not interchangeable:

- **A new eval case is the one thing worth allowing near an auto-gate.** It
  adds a *measurement* and changes no behaviour; `mecha eval` forces MCP, hooks
  and the outbox off and sandboxes mutating cases, so its blast radius is a
  scorecard. It is also the closest safe analogue to what HyperAgents' agents
  actually did — **the loop extending its own verifier**, which is the one kind
  of self-improvement that makes every subsequent proposal better-tested rather
  than more autonomous.
- **A hook is human-reviewed, always.** The dispatch order (interlock → hook →
  approver) means a hook can narrow policy and never loosen it, and `pre_tool`
  fails closed — so its worst case *inside the loop* is denial of service,
  which is loud. But a hook is an arbitrary command with the event on stdin:
  the "cannot loosen" property bounds its effect on the agent, not its effect
  on the machine. Machine-authored code gets read by a person before it runs.
- **A trigger is human-gated absolutely.** It is a scheduled unattended agent
  run — the same reasoning that keeps triggers out of the layered config.
- **An MCP server is never proposed.** Third-party code on the machine.

### 12.3 The limit that stays

HyperAgents' distinguishing move is that the *modification procedure itself* is
editable — metacognitive self-modification. The analogue here is proposing a
change to the diagnostic prompt, and it is deliberately excluded. Every bound
in §11 rests on the diagnostician being handed evidence it did not choose and
being falsified by a measurement it does not run; a loop that can edit that
procedure can relax both. It is the one edit that removes the human from the
last position where a human is cheap.

### 12.4 Is diagnosis a research step?

Yes, in two senses, and the second one has a rule attached.

**It should read, not just think.** The strongest form of the stage is not one
model call over counters — it is an agent with read access to the corpus, the
source, and this repository's own documentation. That last input is the
guardrail rather than the context: `CLAUDE.md` and these research documents
record *why* each mechanism exists and what it cost to learn, which is exactly
what stops a proposal from "fixing" something load-bearing. A diagnostician
that reads "errors neither supersede nor get evicted — a failed call says
nothing about the target" will not propose evicting them; one that sees only
the counters might. **Repository documentation is a safety input to the
proposer, not merely a convenience.**

**It may search the web, and the price is fixed in advance.** External
literature is genuinely useful for diagnosis — most of this document came from
it. But a web-informed diagnosis has read untrusted text, so by §11 its
`Origin` is untrusted and its proposal may only become a *number*, never prose
in the prompt prefix. That is not a restriction on what it may read; it is a
restriction on what its reading may turn into. A human who reads the proposal
can always author the rule themselves — which is the correct place for that
promotion to happen, and the same shape as the front door's split between what
a person may read and what a privileged run may act on.
