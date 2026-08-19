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
