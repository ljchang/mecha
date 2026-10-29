# Harness evolution: what Harness², Hermes and the 2026 field do, and what mecha takes

2026-10-18. Which mechanisms from per-task harness evolution and
self-improving agents mecha adopts, in what shape, and under which of its
existing rules. **Status: proposal. Nothing here is built. R1 and R7 were ruled on
2026-10-18 (§7.1); §7.2 lists the rulings the build still waits on.**

**The problem this answers:** no change that harness rumination has proposed
has ever been accepted. The learning loop works and is not the subject,
except where §5's Tier 2 keeps it honest. §5.0 is the measured case.

## 0. Where this sits

This document builds on four others and restates none of them:

- [`SELF-IMPROVEMENT-RESEARCH.md`](SELF-IMPROVEMENT-RESEARCH.md) holds
  the rules every proposal below has to survive:
  - §5: the input to a prose proposal is a number, never a body of text;
  - §10: best-of-N over a fixed corpus is a multiple-comparisons trap;
  - §14.6: replay can only measure changes that change nothing;
  - §14.7, the owner's ruling: fitness is the owner's own usage, and a
    public benchmark is a transfer check only.
- [`HARNESS-RESEARCH.md`](HARNESS-RESEARCH.md) §9 recommends moving config
  candidates from replay into experiments. R1 (§7.1) answers it in part.
  Behaviour-changing candidates move to measured continuations, but replay
  stays, as the screen for cost-only changes, so §9's "replay was the wrong
  instrument" is adopted only for that class.
- [`SKILLS-RESEARCH.md`](SKILLS-RESEARCH.md) §9 is the standing ruling
  against model-authored skills.
- [`LEARNING-AUTONOMY-DESIGN.md`](LEARNING-AUTONOMY-DESIGN.md) explains why
  learned rules self-accept, and what the counterfactual gate does in place
  of a person.

How it was researched:
- Google Research's `harness2` repository was read in source (§1).
- Nous Research's `hermes-agent` was read in source (§2).
- The 2025–26 literature on harness evolution and on self-curated
  context, with the security work on memory and skill poisoning, was
  surveyed (§3).
- Every mecha symbol cited below was checked against `origin/main` at
  `880ebbc59`.

Evidence strength is marked as in the research docs: 📄 preprint,
🔧 code read, 📰 vendor or secondary.

---

## 1. Harness² — per-task refinement without labels

🔧 [`google-research/harness2`](https://github.com/google-research/harness2)
at `1cfe254` (2026-10-13), Apache-2.0. 📄 Xu et al.,
[arXiv:2610.14224](https://arxiv.org/abs/2610.14224), posted 2026-10-09
(Google Cloud AI Research).

### 1.1 Mechanism

The model's weights stay fixed. What evolves is a tree of eight files, which
the paper calls components:
- **Always-on:** system prompt, guardrails, memory, tool guidance.
- **On demand:** skills, scripts, subagents.
- **Inside the tool loop:** a plugin or hook.

The surface is data (`core/tree.py::SurfaceSpec`), so the same method drives
OpenCode and Codex.

**Task-level recursion (Propose–Probe–Compose)**, `core/recursion.py::run_parallel`:

1. Run the unmodified harness *k* times on the task. These are the base rollouts.
2. An editor model (Opus 4.8 in the paper) proposes **2k candidate
   harnesses from the same evidence**:
   - *k* on the **INV** dimension: how the agent works, meaning procedure,
     tool use and completion;
   - *k* on the **DOM** dimension: what a practitioner knows.

   Each candidate prompt forbids the other dimension's territory
   (`core/prompts.py::_INV_TEMPLATE`, `_DOM_TEMPLATE`).
3. Run each candidate **once, on the same task**.
4. *k* composers each read every trajectory and every candidate diff,
   then write one final harness. A composer may merge candidates, adopt one
   wholesale, or keep the base.
5. Run the finals. A **blind selector** picks one output. Selector ids are
   anonymised by a seeded shuffle (`core/selection.py::assign_ids`), and the
   prompt states that picking an unmodified harness's output is a valid
   outcome.

The budget is asserted as an identity rather than tracked loosely
(`core/ledger.py::Ledger::assert_identity`):
- R = 4k rollouts;
- P = 3k editor sessions.

The sequential mode (`run_sequential`) carries the composed checkpoint
through *k* rounds instead of widening.

**Harnessing-level recursion** (`core/selection.py::reflect_adoption`,
`core/library.py::Library`) runs after selection:
1. A reflector reads the whole record and writes per-edit rows. Each row
   gets a verdict from `helped`, `hurt`, `no observed effect` or
   `confounded`, and the prompt says to *prefer* `confounded` when one
   rollout is all that stands behind a difference.
2. It then writes at most one lesson as `cue / lesson / evidence /
   confidence`, or `NO_ENTRY`.
3. The adopted tree and the lesson are stored **per domain**, so retrieval
   is the directory: a domain's folder holds only that domain's entries.
4. Later tasks in the domain read the index before proposing.

### 1.2 What the evidence says (📄, self-reported)

- **Headline at k=1:** +11.5 on WorkBuddy-Bench, +4.5 on JobBench and
  +11.8 pp on LAB partial pass, over each base harness. The executors are
  GLM-5 and Gemini 3.5 Flash.
- **Ablations** (Gemini 3.5 Flash, OpenCode, k=1, Table 3):

  | Ablation | WorkBuddy | JobBench |
  |---|---|---|
  | Without the library | −3.1 | −1.8 |
  | Without the INV/DOM split ("scoping") | −2.9 | **−3.7** |
  | Without composition | −1.6 | −1.9 |

  The split is the largest single effect on JobBench.
- **Which components carried the gain.** Each was removed from an improved
  harness (Table 6, WorkBuddy):

  | Removed | Average loss |
  |---|---|
  | Skills | −5.3 |
  | Scripts | −3.7 |
  | Always-on prompt text | −1.5 |

  Every ablated harness still beat the base, and the authors say the effects
  are not additive.
- **Cross-model transfer.** A harness refined for Gemini 3.5 Flash was run
  unchanged:

  | Executor | Code | Office |
  |---|---|---|
  | GLM-5 | +17.3 | — |
  | Gemini 3.7 Flash | +0.3 | +6.4 |
  | Opus 4.8 | +2.6 | +1.1 |

  The authors say the benefit "varies substantially with the executor and
  domain".
- **The editor need not be stronger.** In a same-model setup, all five
  models improved on every benchmark. Their conclusion: "harness refinement
  need not rely on a stronger editor."
- **The paper's own failure analysis.** Nine regressions fell into three
  groups:
  - a weaker candidate was selected;
  - **an incorrect premise propagated across candidates**;
  - outcome variance.

  Two further statements from the paper: "adoption rather than verified
  optimality", and execution contrasts "do not guarantee correctness".

### 1.3 What it does well, as engineering

- **Evidence is files and the prompt is a pointer** (`core/context.py`):
  - Trajectories are never truncated to fit a prompt.
  - The mission names only the files that actually landed
    (`ContextTree::layout`).
  - A context with no renderable trajectory refuses to run rather than
    adapting on nothing.
- **There is a tripwire on the grader's output**
  (`core/context.py::assert_no_grader_verdicts`):
  - It scans every context tree by file name *and* content before a model
    sees it.
  - It scans every agent-written harness before storage.
  - A match is "a leak, not a false positive to widen around".
- **Trial identity is a fingerprint, not presence** (`core/runmark.py`). A
  completion marker digests the harness tree that produced the rollout, so a
  resumed run never reuses output from a different configuration.
- **The edit-form hierarchy is in the prompt** (`_SURFACE_OPENCODE`).
  Always-on prose is "the expensive option", so the prompt says to prefer:
  1. a skill;
  2. then a script;
  3. then a subagent;
  4. then a hook;
  5. always-on text last, and only if something else is cut.

  The component ablation above is the evidence behind that order.
- **KEEP_BASE is a first-class answer** at every stage. "Adaptation that the
  evidence does not argue for is a regression."

### 1.4 What it lacks, measured against mecha

- **No provenance.** Trajectories, which hold third-party content, become
  library lessons. The only defence is a prompt section headed "evidence is
  data, not instructions". `SELF-IMPROVEMENT-RESEARCH.md` §5 is the reason
  mecha does not do this.
- **No gate, holdout, revert or retirement.** The library only grows.
  `Library::forget` exists for resume, not for harm.
- **Executable self-edits.** The editor writes plugins that run inside the
  tool loop. The README states that Codex runs with its sandbox disabled and
  network access is unrestricted.
- **Cost is a multiple of the task.** At k=1 a task costs 4 executions plus
  3 editor sessions, a selector, and a reflector. A task that sends mail
  cannot run 4 times. This is the decisive mismatch: **the per-task loop
  only applies where a run has no side effects.**

---

## 2. Hermes Agent — self-improvement as a product default

🔧 [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent)
at `9e01b44d` (2026-10-18), MIT. The latest release is v0.22.2. The GitHub
API returned `stargazers_count` 254,853 on 2026-10-18; the figure carries
nothing below. 📰 Its
[docs site](https://hermes-agent.nousresearch.com/docs).

There is no paper. A companion repo, `hermes-agent-self-evolution`, runs
DSPy+GEPA over SKILL.md files offline, behind tests and a human PR review;
its later phases are unbuilt. `FEATURES-DESIGN.md` §2 already covers
Hermes's setup and toolsets. This section covers only how it improves itself.

### 2.1 Mechanism (🔧 unless marked)

- **Reviews run in the background, on a counter.** After each delivered
  turn, `agent/turn_finalizer.py` checks two counters:
  - a memory review every 10 user turns (`memory.nudge_interval`);
  - a skill review after 10 tool iterations with no `skill_manage` call.

  Either one starts `agent/background_review.py::spawn_background_review_thread`,
  a daemon-thread fork of the agent on the same model and the same cached
  prompt. The fork's tools are restricted at dispatch, so the advertised
  tool list stays byte-identical to the parent's. It gets up to 16
  iterations.
- **The review prompt pushes for output.** "Be ACTIVE — most sessions
  produce at least one skill update", and "a pass that does nothing is a
  missed learning opportunity".

  Its counterweight is `_DO_NOT_CAPTURE_BLOCK`, a list of what must never
  become a skill:
  - environment-dependent failures, which "become persistent
    self-imposed constraints that bite you later";
  - negative claims about tools;
  - one-off narratives;
  - unresolved failures presented as workflows.
- **The model writes skills, including executable scripts.**
  `tools/skill_manager_tool.py::skill_manage` creates, patches, edits and
  deletes SKILL.md trees, along with their `scripts/`, `references/` and
  `templates/`.
  - A read-before-write guard refuses a review fork's patch to a file it
    did not read during that review.
  - The fork may patch any skill, including bundled and hub skills.
- **Provenance means the author, not the content.**
  `tools/skill_provenance.py` records whether a skill was written in the
  `foreground` or by a `background_review`, and nothing about what the
  conversation had read.
- **Memory is two capped files**, frozen into the prompt at session start:
  - `MEMORY.md` at 2,200 chars and `USER.md` at 1,375 chars;
  - writes are regex-scanned (`tools/threat_patterns.py`);
  - an unattended fork may only add — a patch or a removal is staged for
    the owner (`memory_tool._background_delete_gate`).
- **A curator runs when the agent is idle** (`agent/curator.py`), weekly
  and after two idle hours:
  - It marks a skill stale after 14 days without use and archives it at 30.
  - Zero uses is treated as absence of evidence, so a young skill is never
    aged out for being unused.
  - Pinned and cron-referenced skills are exempt.
  - An LLM consolidation pass is off by default, backs up first, and never
    deletes.
- **There is an audit ledger, not a gate.** `tools/skill_ledger.py` writes
  content-addressed before and after blobs per mutation, with single-edit
  rollback, and calls itself "TELEMETRY, NOT A GATE".
- **Nothing measures whether a skill or a memory helps.**
  `tools/skill_usage.py` counts uses, views and patches.
- **The cached prefix is protected.** The system prompt is byte-stable for a
  conversation. A mid-session skill change arrives as a note appended to the
  next user message (`agent/skills_index_delta.py`), never as a prompt
  rebuild.

### 2.2 Its security posture (🔧 `SECURITY.md` and source)

- **"The only security boundary against an adversarial LLM is the
  operating system."** The approval check, the skill scanner and redaction
  are declared "not boundaries", and prompt injection itself is out of scope.
- **Approval is off by default and fails open.**
  `tools/write_approval.py::write_approval_enabled` reads an unset or
  invalid value as "gate off", and a config that cannot be read returns
  `False`. This is the silently-degrading-guard shape.
- **Agent-written skills are not scanned by default**
  (`skills.guard_agent_created` is false).
- **There is no conversation-level taint.** A tool result is wrapped as
  untrusted only by tool name (`agent/tool_dispatch_helpers.py::_maybe_wrap_untrusted`).
  The review fork reads that conversation and writes persistent prompt
  text with no check on what it read.
- **Hub trust is a repo-name allowlist plus a regex scan.** 📰 There is one
  CVE against the skill scanner (CVE-2026-9353), and open issues on
  unscanned skill descriptions and on stale memory poisoning resumed
  sessions.
- 📰 **Reception.** The main complaint is skill bloat: near-duplicates and
  context inflation. The curator was the answer to it. An independent
  hands-on test on a local 30B model found three near-duplicate skills and
  concluded "human review still needed". PASB (`PERSONA-MEMORY-TIMESCALES-RESEARCH.md`
  §3.2) measured Hermes saving a user's passing claim as a lasting fact in
  51.4% of runs.

### 2.3 Read against mecha

Hermes is the existence proof that a self-writing skill library ships at
scale on local models. It is also the clearest instance of the design
mecha's rules forbid: a model writes executable, persistent prompt material
out of conversations it never classified, behind gates that are off by
default.

The parts worth taking are the ones that do not depend on that:
- the do-not-capture list (P7);
- the add-only rule for unattended writers, which mecha already keeps in a
  stronger form;
- absence of use treated as absence of evidence, never as grounds on its
  own;
- appending a change to the tail instead of rebuilding the prefix (P5).

---

## 3. The rest of the field

### 3.1 Harness-evolution papers

📄 All preprints. Each was read as the arXiv HTML body unless marked
*(abstract)*; appendices were sometimes truncated. AHE is already in
`SELF-IMPROVEMENT-RESEARCH.md` §3 and is listed here for comparison only.
**The affiliations given for Evo² and Harmony in secondary coverage are not
confirmed on the arXiv pages.**

| Method | Edits | Accept signal | Holdout / rollback | Key limitation |
|---|---|---|---|---|
| Meta-Harness [2603.28052](https://arxiv.org/abs/2603.28052) | A one-file harness | Ground truth on a search set; Pareto frontier with context cost | Held-out sets for text and maths; on TB2 the search set *is* the eval set | Selects on what it reports |
| Evo² [2610.13025](https://arxiv.org/abs/2610.13025) | Harness source **and weights** (SFT + GRPO on the proposer/reflector) | Δ benchmark score | Split not explained | Weight-only SWE gain p=.086 |
| Harmony [2610.15150](https://arxiv.org/abs/2610.15150) | Whole harness repo, general vs benchmark-specific parts | Unit tests + exploit PoCs; an LLM overfit judge that sees only the diff | Evolve / held-out-ID / held-out-OOD; restore last valid parent | **Edited the evaluator when it was writable** |
| Harness² [2610.14224](https://arxiv.org/abs/2610.14224) | Eight file components | Label-free behaviour contrast | None | §1.2 |
| HarnessLens [2608.27311](https://arxiv.org/abs/2608.27311) | Harness components | **Paired live rollouts labelled Recovered / Stable / Regressed**, plus a fresh confirmation batch | TEST untouched | Task selection uses an LLM |
| Rethinking harness evolution [2607.12227](https://arxiv.org/abs/2607.12227) | Per-task revision | Unit tests or none | Held-out 45/10/34 split | **Evolution does not beat matched-budget sampling on TB2.1**; gains were "memorised fixes" |
| TTHE [2607.08124](https://arxiv.org/abs/2607.08124) | Harness code at test time | Label-free proxies (execution health, round-trip consistency) | None | Committed 50% against a final-pool oracle of 64%: **selection is where it loses** |
| OpenJarvis [2605.17172](https://arxiv.org/abs/2605.17172) | Typed spec: model, engine, agent, tools/memory, learning | **The target failure cluster improves and no other cluster drops past a tolerance** | Rollback | Its cloud teacher sees scrubbed traces |
| HSI [2608.08466](https://arxiv.org/abs/2608.08466) | Harness + evolver; outer anchor immutable | Lower confidence bound on reward (μ − zσ/√n) | 20% held out; branch back | Non-monotonic: the best iterate came early |
| JIT-Agent [2608.25593](https://arxiv.org/abs/2608.25593) | A trained generator writes per-task harnesses | Task reward | None described | Training cost unreported |
| Harness-Zero [2609.24974](https://arxiv.org/abs/2609.24974) | Weights (LoRA); the harness is distilled away | A stronger reviewer corrects each step | Held-out splits | Context management cannot be distilled |
| ClawGym II [2608.16798](https://arxiv.org/abs/2608.16798) | Weights; the harness is a black box | Verifier + rubric | Separate benchmarks | No audit for reward hacking |
| DGM [2505.22954](https://arxiv.org/abs/2505.22954) / HGM [2510.21614](https://arxiv.org/abs/2510.21614) *(abstract)* | Own codebase | Staged benchmark | Archive, sandbox | Objective hacking; ~$22k a run |
| SICA [2504.15228](https://arxiv.org/abs/2504.15228) | Whole codebase, including its overseer | Weighted score / cost / time | Archive | 50-task subsets |

Five findings across the table bear on the proposals:

1. **Label-free selection is where these methods lose.**
   - TTHE commits a 50% answer from a pool whose best is 64%.
   - Harness²'s failure analysis names a weaker candidate selected, and a
     false premise shared across candidates.
   - The editor and the judge share a model's biases.

   This is the case against auto-accepting anything on judge or proxy
   evidence, and for mecha's gate staying numeric.
2. **Matched budgets erase a good part of the gains.** On TB2.1, harness
   evolution was no better than spending the same budget on more samples.
   **Mecha's gate has no null arm**, so it cannot say how often a candidate
   "wins" against an unchanged copy of itself (P2).
3. **Writable evaluators get edited.**
   - Harmony's agent rewrote its score file and reported 100%.
   - A tampering audit ([2609.00069](https://arxiv.org/abs/2609.00069))
     found at least one finding in 63–85% of iterations for DGM, ADAS and
     HyperAgents.

   No model-authored change may ever reach the measurement path (§6).
4. **Prompt-only edits are the weakest lever.**
   - AHE's prompt-only edits regressed, from 69.7 to 67.4.
   - Harness²'s component ablation ranked always-on prose last.
   - Structural edits transferred where prose did not.

   This supports staging prose for a person, and putting knowledge where it
   is cheapest (P5).
5. **The gates that hold up check regression per cluster and confirm on
   fresh pairs.** OpenJarvis gates on "target cluster up, no other cluster
   down past a tolerance". HarnessLens's metric-only gate would have
   accepted 10 of 19 iterations against its own 5, and the extra acceptances
   did not hold. Both are closer to mecha's gate than to Harness², and both
   are evidence for P8.

### 3.2 Self-curated context, memory and skills

ACE, MINJA and PASB are already in `SELF-IMPROVEMENT-RESEARCH.md` §4,
`MEMORY-RESEARCH.md` and `PERSONA-MEMORY-TIMESCALES-RESEARCH.md` §3.2. This
section adds what is new since then, and what bears on the proposals.

| System | Who writes | What validates a write | Pruning | Strength |
|---|---|---|---|---|
| ACE ([2510.04618](https://arxiv.org/abs/2510.04618)) | A curator emits deltas; non-LLM code merges them | Labels or execution feedback | Embedding dedup, plus helpful/harmful counters per bullet | 📄 ICLR 2026 |
| GEPA ([2507.19457](https://arxiv.org/abs/2507.19457)) | A reflective mutator, offline | Metric plus feedback; a **Pareto frontier per instance** | The frontier | 📄 ICLR 2026 |
| ReasoningBank + MaTTS ([2509.25140](https://arxiv.org/abs/2509.25140)) | The agent's own model, up to 3 items per trajectory, failures included | An LLM judge, admitted noisy | None; append-only | 📄 ICLR 2026 |
| Letta sleep-time ([2504.13171](https://arxiv.org/abs/2504.13171)) | A **separate** sleep-time agent; the primary cannot edit its core memory | None | `memory_rethink` | 📄 / 📰 |
| Codex memories ([docs](https://learn.chatgpt.com/codex/memories)) | A background pass over idle chats | Secret redaction; **`disable_on_external_context`** excludes chats that used MCP or web | Consolidation | 📰 |
| Anthropic Agent Skills ([eng. blog](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)) | People, with Claude capturing drafts | Evaluations first; agent self-authoring is listed as *future* | — | 📰 |

Three findings change how much weight a proposal below can carry:

- **Self-generated skills do not help on average.**
  - SkillsBench ([2602.12670](https://arxiv.org/abs/2602.12670), 📄)
    measured curated skills at +16.2 pp but self-generated skills at
    roughly zero: −1.3 pp in the NeurIPS version. **18 of 84 tasks regressed
    even with curated skills.**
  - [2605.23899](https://arxiv.org/abs/2605.23899) (📄) finds "non-trivial
    negative transfer" from experience-derived skills, and finds that a good
    extractor can be a poor consumer.

  Both corroborate `SKILLS-RESEARCH.md` §9. They also say interactions
  between rules are where harm hides, which argues for testing the set
  (P3), not only each rule.
- **Retrieving more memories can make things worse.** ReasoningBank's
  success *fell* when it retrieved 2–4 items instead of 1: conflicting items
  add noise. This is the mechanism behind mecha's 25-rule and 2,600-char
  per-domain caps, so the caps are justified, not just tidy.
- **Whole-set rewriting collapses context.** ACE measured Dynamic
  Cheatsheet's rewrite taking 18,282 tokens down to 122, with accuracy
  falling below the no-memory baseline. ACE also served 91.8% of input from
  the prompt cache because its playbook changes by deltas. **Mecha's
  learner still rewrites whole regions**. In `learning.rs`, the doc comment
  that opens "Mint identity for a freshly learned rule set" says: "The
  learner rewrites whole sets, so identity has to survive the rewrite". §5 P4 names one concrete cost of that.

### 3.3 The security record on memory and skill poisoning

The new evidence since `SELF-IMPROVEMENT-RESEARCH.md` §5, all 📄 unless
marked:

- **MemoryGraft** ([2512.16962](https://arxiv.org/abs/2512.16962)) forges
  "successful experiences" from a README. A few grafted records dominate
  retrieval because the agent imitates past success.
- **FARMA** ([2607.05029](https://arxiv.org/abs/2607.05029)) forges
  reasoning traces that reinforce one another and reaches 100% attack
  success.
  - It defeats keyword filters, and it defeats A-MemGuard's consensus check
    ([2510.02373](https://arxiv.org/abs/2510.02373)).
  - **Consensus is defeated by amplification.** Support counted from
    entries the store itself produced is not independent support.
- **The delayed tool invocation attack on Gemini** (Rehberger, 2025, 📰).
  The injection makes the memory write wait until the user's next "yes",
  so the write looks user-initiated. **This is the attack aimed at learning
  from human interventions**, mecha's only source of reflections. The
  defence is the one mecha already has: taint belongs to the whole
  conversation, not to a turn, so a clean-looking "yes" in an armed
  conversation is never mined. That property must never become per-turn.
- **SpAIware and ZombieAgent** (📰 2024, Jan 2026): mail or a file plants
  rules in ChatGPT's memory, and they exfiltrate data in every later
  conversation. These are the production instances of Zombie Agents.
- **Skill marketplaces are hostile.**
  - ClawHavoc (📰 Feb 2026): 341 of 2,857 audited ClawHub skills were
    malicious, most delivering a stealer through fake "Prerequisites"
    sections.
  - A study of 31,132 skills ([2601.10338](https://arxiv.org/abs/2601.10338))
    found 26.1% vulnerable, with skills that bundle scripts about twice as
    likely to be.
  - A Socket audit of OpenClaw's community `self-evolve` skill (📰) found it
    designed to bypass approval and rewrite its own governing files.
- **Lineage works as a defence.** MemLineage
  ([2605.14421](https://arxiv.org/abs/2605.14421)) gates sensitive actions
  on each memory's provenance and drove attack success to zero in its
  harness. Codex's `disable_on_external_context` is the product version of
  mecha's clean-only mining.

Read against mecha, the existing combination covers MINJA, MemoryGraft,
delayed invocation and the skill-hub vector by construction:
- taint held by the whole conversation, with no knob;
- reflections only from human interventions;
- the counterfactual gate;
- retirement;
- user-authored skills only.

Two gaps remain, and §5 takes both:
- **Lineage is recorded but never re-checked.** `Rule::sources` names the
  reflections a rule came from, but a later change to a source's standing
  does not reach the rule (P6).
- **Support is not checked for independence.** Nothing stops a rule's
  support from being several interventions out of one session (P6).

---

## 4. Mecha against the field

| | Harness² | Hermes | The 2026 papers | mecha today |
|---|---|---|---|---|
| **What evolves** | Eight harness files, per task | Skills (with scripts) and two memory files | Code, prompts, workflows; some also weights | Four knobs (`harness::OverrideKey`: three numbers and `effort`) and learned rules |
| **Who writes it** | An editor model | The agent's own background fork | A proposer model | The diagnostician proposes knobs; the learner writes rules from reflections |
| **What accepts it** | Blind selection; no verifier | Nothing (approval is off by default) | Ground truth, tests, judges or proxies | A paired replay gate with a holdout, work floor and regression ceiling (`candidate::judge_drawn`); a counterfactual gate for rules |
| **Holdout** | None | None | Some | Yes, drawn uniformly before diagnosis |
| **Rollback** | None | A per-edit ledger | Some revert | `harness revert`; rule retirement on measured regression |
| **Provenance of what it learns from** | None | Records the author, not the content | None discussed | Conversation-level taint; clean only, no knob |
| **Executable self-edits** | Yes (plugins, scripts) | Yes (skill scripts) | Yes, mostly | None |
| **Prefix** | Not its concern | Frozen per session; deltas in the tail | Not discussed | Byte-stable; nothing toggles per turn |
| **Measures behaviour change** | Yes, live, but unverified | No | Yes, against benchmarks | **No.** Replay drops behaviour-changing candidates (`SELF-IMPROVEMENT-RESEARCH.md` §14.6) |

The table shows a mirror image:
- **The field** measures behaviour change live and gates it weakly or not at
  all.
- **Mecha** gates rigorously but cannot see behaviour change.
- **Hermes** does neither, and ships.

**The two halves of mecha are not in the same state.** The learning loop
works: it mines, gates, carries and retires rules. Harness rumination has
never had a suggestion accepted. §5.0 measures why, and the proposals are
ordered by that diagnosis rather than by size.

---

## 5. Proposals

### 5.0 Why harness rumination has produced nothing, measured

`mecha harness list --all`, read 2026-10-18: **17 candidates since
2026-08-25, none accepted.** 12 were rejected and 5 are staged.

| Outcome | Count | Candidates |
|---|---|---|
| Named a setting outside the closed set: three invented keys, one security switch (all before the late-August fixes) | 4 | `tool.validation.strict`, `context.auto_compact`, `context.compaction.threshold`, `security.minimize_taint` |
| Measured with no discriminating power (all ties on a scripted smoke test). The gate staged them below the floor; the owner rejected them by hand with "NOT a verdict on the knob … worth re-proposing" | 3 | `compact_at_tokens`, `max_output_tokens`, `effort` |
| Judged useless or harmful by the owner | 3 | `max_turns=10` ("does nothing"), `max_turns=3` ("harmful"), `max_output_tokens=4096` ("backwards") |
| Untested | 1 | `effort=low` |
| An architecture change contradicted by its own brief | 1 | retained-failure eviction |
| Staged config, below the floor or unmeasurable | 3 | the latest: "16 diverged" on *both* arms |
| Staged architecture, waiting for a person | 2 | 503 retry; semantic boredom |

Three causes, each with its own evidence:

1. **The proposer is blind.** The brief is "built from counters, not
   content" (`diagnose.rs`), so the diagnostician has never seen a failure,
   only a count of failures. What it produced is what blind proposals look
   like:
   - keys invented to fit a number. This one is fixed: `DIAGNOSE_INSTRUCTION`
     now names the closed set, `OverrideKey::parse` refuses a key outside it,
     and `diagnose::names_guarded_setting` catches a security key. The 13 candidates since are the current evidence;
   - a mechanism "reverse-engineered onto" the one counter with a nonzero
     mean;
   - extreme values of the only knobs it was allowed.

   Harness²'s editors must "cite the specific trajectory moment behind
   every change; one you cannot trace to a moment is speculation". Mecha's
   diagnostician cannot cite a moment, because it is never shown one. This
   is a choice of the brief. Whether reading more is allowed is P9's
   question: `SELF-IMPROVEMENT-RESEARCH.md` §13.1 lets the diagnostician
   read *web pages* under an artifact test, but transcripts fall under §5's
   input rule, which P9 argues from and R6 decides. **P9.**
2. **The instrument cannot see.** On a non-greedy local sampler the
   *baseline* arm diverges from its own recording about as often as the
   candidate. The newest candidate shows 16 divergences on each arm, and
   `HANDOFF.md` records "ten of eleven divergences … were baseline-arm" on
   2026-09-03. Replay therefore cannot tell a change that moved behaviour
   from noise, so it can neither accept nor reject on the merits. **P8,
   with P2 to count the noise.**
3. **The surface is where the gains are not.**
   - Four knobs (three numbers and `effort`) are all the loop can apply. In every study in §3
     that ablates components, the gains sit in structure: skills, scripts,
     tool guidance, middleware. Harness²'s prompt-only ablation cost 1.5
     points against 5.3 for skills, and AHE's prompt-only edits regressed.
   - The two structural candidates the diagnostician did produce are staged
     with no evidence attached, because nothing measures architecture.
     "Architecture is always a human's call" is right; a human's call made
     with no measurement is a coin the owner has to flip.

   **P10.**

The learning loop does not share these causes:
- its sensor is the owner's interventions, which are content, from clean
  sessions;
- its instrument branches at the intervention rather than replaying whole
  runs;
- its surface is prose that a run actually reads.

That is why it works, and why P3–P7 are upkeep, not rescue.

**So the proposals come in two tiers.**
- **Tier 1 makes harness improvement effective:** P8 (an instrument that
  sees behaviour, built as `ROLLOUT-DESIGN.md`'s continued branches), P9 (a
  proposer that sees failures) and P10 (a surface where the gains are). P1
  and P2 are their small prerequisites: an honest history and a counted
  noise floor. P11, preplay, is the same engine run online.
- **Tier 2 keeps a working learning loop honest:** P3–P7.

Each proposal states:
- the gap, with the mecha symbol that shows it;
- where the idea comes from;
- the shape;
- why it keeps the rules in §0.

**Every gap was checked against `880ebbc59` by reading the code.** None has
yet been reproduced by a failing test, and doing that is the first step of
each build.

### P1. A past candidate's verdict is typed, not prose — S

**Gap.**
- `candidate::Guard` already separates "did not beat the original" (a
  missing win) from "made something worse" (a regression). The
  `Judgement::guard` field's doc comment calls them "opposite findings".
- `commands/diagnose.rs::harness_history` throws that away. It renders each
  past candidate's free-text `reason`, falling back to "measured worse", and
  `diagnose::Evidence` heads the list with "do not propose any of these
  again; a measured rejection is evidence, not an invitation to retry".
- So an all-ties rejection (`judge_drawn`: "did not beat the original: 0
  better, 0 worse, N unchanged") reaches the next diagnostician as a ban.
  On an instrument that drops every behaviour-changing pair (§14.6), all
  ties is the *expected* result for most candidates.
- `HANDOFF.md` records the incident: three of the four overridable knobs
  measured all ties with identical work on both arms, and were "one
  careless reason away from retirement on evidence that measured nothing".
- The fix on `fix/selection-slice-power` (`cbc3738`, unmerged, the owner
  has not decided) stops the gate *rejecting* identical arms. P1 is the
  other half: it stops the *history* turning whatever verdict the gate gave
  into a ban.

**Source.** Harness²'s reflector labels each edit `helped`, `hurt`,
`no observed effect` or `confounded`, and prefers `confounded` whenever one
rollout is all the evidence (`core/prompts.py::experience`).

**Shape.**
- Derive one verdict per past candidate from its stored `Measurement`,
  never from its prose:
  - `harmed`: a loss, a guard breach, or the work floor;
  - `no observed effect`: ties, or wins not above losses;
  - `underpowered`: below the floors;
  - `owner`: rejected or reverted by hand.
- **The owner's verdict is typed too.** `mecha harness reject` takes one
  of `harmful`, `no-effect` or `unmeasured` beside its free-text reason, and
  the owner group renders by that verdict. The three all-ties knobs in
  §5.0 are why: the owner rejected them while writing that they were "NOT
  a verdict on the knob", and an "owner rejections are never re-proposed"
  rule would contradict the owner's own reason.
- Render the history in those groups, with a different instruction for
  each:
  - `harmed`, and an owner verdict of `harmful`, are never re-proposed;
  - `no observed effect` may be re-proposed once the instrument or the
    corpus has changed since it was measured. The brief states which,
    if either, has changed (a new replay corpus window; P8, once it
    exists). Until then the instruction says plainly that nothing has
    changed, so it is never a promise P8 has not kept yet;
  - `underpowered`, and an owner verdict of `unmeasured`, are silent on
    retrying.

**Rules kept.** Counters in, no prose. The prompt sits in the
diagnostician's brief, not in any run's prefix.

**Built (2026-10-19)** as `harness::HistoryVerdict`, with three decisions
the shape above left open:
- **An owner rejection with no typed verdict is still never re-proposed.**
  `reject` without `--verdict` is a plain no. The owner can type an old
  rejection later (`reject <id> --verdict unmeasured` on one already
  rejected), and only that changes.
- **The re-derivation check reads the same verdict.** It used to refuse any
  change already on file. Left alone, the brief would invite back changes
  that the loop then refused the night they returned.
- **The gate's guard is now stored with each measurement.** A breach on a
  metric the candidate did not predict rejects without showing in the
  tallies. A gate rejection recorded before this field is untyped, never
  "no observed effect".

**Almost nothing reopens on its own.** A thin selection slice always
proposes and never rejects, so measuring can't produce `Underpowered`. Only
two things reopen a change:
- **The owner's `--verdict unmeasured`.**
- **A no-headroom rejection (`HistoryVerdict::NoHeadroom`)**, where no run in
  that night's corpus had any of the metric the change predicted to lower.
  Headroom belongs to the corpus, not the change. The rejection holds until
  the counters show that metric above zero. The brief says so, and the
  re-derivation check applies the same `forbids_retry`.

Apart from those, P1 lifts only the bans the owner lifts. The brief shows
each change once, by the record that decides it, so a retried change never
appears under two contradicting instructions.

**A held setting is held at every value** (2026-10-29). The re-derivation
check matched the whole `KEY=VALUE`, so a staged `max_turns=20` banned
`max_turns=20` and nothing else. From 10-09 to 10-28 the nightly proposed
`max_turns` at 5, 4, 20, 100, 200, 60 and 25, each staged with the arms
identical or too few to measure (`mecha harness list --all`). `HarnessCandidate::forbids_proposal`
now also refuses a different value of a setting while a record for it is
`Staged`, `NoObservedEffect` or `OwnerNoEffect`
(`HistoryVerdict::holds_setting`; `HarnessCandidate::held_setting` is the
one predicate the brief reads too). The brief's two groups say so, a held
setting shows once by its newest holder, other values of it are not shown as
proposable, and the holder survives the newest-20 window. A harm
stays the exact value's: `max_turns=3` harmful does not say 30 is. Not done:
checking, before the replay is paid for, that any drawn episode could reach
the new limit.

The live record when it was built: the gate had rejected none of the 17
candidates. All 12 rejections were the owner's, and four of them were keys
that never existed.

### P2. A null arm, and tenure for accepted overrides — S/M

**Gap.**
- The gate counts paired wins (`MIN_SELECTION_PAIRS` = 8,
  `MIN_HOLDOUT_PAIRS` = 4) but has never measured how often an
  **unchanged** configuration "wins" against itself. Replay on a non-greedy
  local model is pass@k-shaped (`ARCHITECTURE.md`, the replay notes).
  `HANDOFF.md` already holds the informal version of this measurement:
  "the baseline arm diverges on most non-scripted episodes … because a
  local sampler does not reproduce its own tool sequence". The noise floor
  exists. It just has never been counted.
- An accepted override is permanent until a person reverts it. Learned
  rules, by contrast, are re-validated and retired ("acceptance is not
  tenure"). `tenure.rs` is a different mechanism: a rule's promotion-only
  standing with the owner. Retirement is what carries the contrast.

**Source.**
- Rethinking harness evolution: gains vanish at a matched budget.
- HSI: the best iterate came early, and later ones regressed.
- DGM's tampering audit: early defects persist down the lineage.

**Shape.**
1. An A/A run through `harness_probe` whose "change" is the identity, on a
   schedule. The result is recorded beside the floors, so the floors are
   justified by a number rather than asserted. If an identity arm clears
   the floors at any meaningful rate, the floors are wrong.
2. Every accepted override is periodically re-measured against its own
   absence, with the same gate and arms swapped. Removing an override only
   ever moves toward defaults, which is a narrowing. Whether it reverts
   automatically or stages for a person is **R4**.

**Built (2026-10-19): step 1 only.** `mecha harness null` and `mecha harness
floors` exist. A null run uses `ruminate`'s draw and the same `drive_arms`
loop with nothing changed on either arm, and judges the pairs under every
metric. It is not scheduled: the owner adds the trigger, because a null run
costs as much replay as a real measurement. The point-wise comparison's own
noise floor is not measured, since it needs a change to apply at each
decision point. Step 2 waits on R4, and no override has ever been accepted
that it could re-measure.

### P3. The learn gate also probes interventions it did not learn from — M

**Gap.**
- `commands/learn.rs` measures a candidate rule set by branching only at the
  batch's own reflections: the loop under "the gate: measure the candidate"
  iterates `reflexions`. A rule is graded on the very cases it was derived
  from.
- Regressions on *other* interventions surface only later, in `mecha
  validate`, after the set is live in every prompt of the domain.

**Source.**
- GEPA's per-instance Pareto frontier: a candidate must not lose any case,
  not merely win on average.
- SkillsBench: 18 of 84 tasks regressed even under curated skills.
- Meta-Harness and AHE both select on the set they report.

**Shape.**
- Before applying, also probe a uniform draw (`sample.rs`) of the domain's
  earlier interventions whose last validation was clean, current block
  against candidate block.
- Any regression refuses the batch, exactly as a regression on its own
  reflections does today.
- Report this tally separately, as the holdout it is.
- The cost is a bounded number of extra probe pairs, at night.

**Rules kept.** It narrows only: more ways to refuse, none to accept.

### P4. A rule's record survives being reworded — M

**Gap.**
- `learning::finalize_rules` carries a rule's identity forward **only on
  exact text** — deliberately, so that two distinct rules cannot merge.
- Convictions are keyed by id (`ValidationRecord::attributed_rule_id`,
  `rule_tallies`).
- The retirement brake catches a reworded *retired* rule. For an *active*
  one, `learning::near_restatements` **detects** a near miss: the same text
  once case, punctuation and the `-ise`/`-ize` spelling are normalised.
  `commands/learn.rs` warns on it ("restates … but not word for word"),
  and carries nothing, by design: "Reported, never merged".
- A genuine paraphrase in new words is not detected at all.
- Either way, a rule with two attributed regressions, one short of
  `DEFAULT_RETIRE_AT`, comes back under a fresh id with an empty record,
  and its probation is re-decided by the new batch.

**Source.**
- ACE's delta protocol (ADD / UPDATE / REMOVE on stable bullet ids, merged
  by non-LLM code), and its measurement of whole-set rewriting collapsing
  context.
- Hermes's content-addressed per-mutation ledger.

**Shape, two options (R2).**
- **(a) Deltas.** The learner replies `keep <id>`, `update <id>: <text>`,
  `add: <text>` and `retire <id>`:
  - an `update` keeps its id, tallies, probation and lineage;
  - untouched rules are untouched by construction;
  - the reply *is* a per-mutation ledger.
- **(b) The narrow fix.** Where `near_restatements` already pairs a new rule
  with an active one, carry the old rule's convictions and probation to the
  new id, and leave identity alone. The detection exists, so this is the
  carry only. It covers near misses and no paraphrase.

(a) is the structural answer to `SELF-IMPROVEMENT-RESEARCH.md` §4, and the
only option that covers a paraphrase. (b) is a day's work and closes the
near-miss half.

### P5. Knowledge that fires on an error, in the tail — M

**Gap.**
- Learned rules and skill listings ride always-on in the cached prefix
  (`RULES_CHAR_BUDGET` = 2,600 per domain).
- `post_tool` hooks only observe (`hooks.rs`).
- So knowledge that matters only after a particular tool failure is paid
  for on every run of the domain, or not carried at all.

**Source.**
- Harness²'s most-used plugin shape: "appending a targeted hint after an
  error signature that keeps recurring".
- Its component ablation: always-on prose came last.
- Hermes's tail delta: change the end of the conversation, never the head.

**Shape.**
- A hint record keyed on **tool name plus an owner-written match** against
  the error result.
- When a result with `is_error` matches, the hint is carried as **its
  own non-external block beside the result**, never concatenated into it.
  It rides in the message that carries the tool results, the slot steering
  text already uses, so it adds no new message shape and every `tool_use`
  id still gets exactly one result.
  - "Beside" is not literal. A turn's `tool_result` blocks come first (the
    loop already appends tool pictures after every result for this
    reason), so in a batch the hints follow all the results.
  - Each hint therefore carries a caption naming the call it explains,
    the way the picture path captions its images.
  - Concatenated into an `external: true` result, owner text would ride
    inside a block the conversation labels third-party. That is the
    inverse of the incident `CLAUDE.md` records, where a guard's refusal
    was labelled third-party.
  - A delimiter inside the result would be the same prompt-level defence
    §1.4 faults Harness² for.
  - A separate block keeps each label honest. It is also why R3 should
    land on owner-written text only: a counter-templated hint would sit in
    the identical position an injection would.
- It sits in the moving tail, so it costs the prefix nothing and costs
  nothing on runs where the error never fires.
- The result keeps its own `external` flag, and the note's block is
  harness-origin: the note neither launders nor adds taint.
- Hints live in `~/.mecha/` with triggers and skills, never in layered
  config, so a cloned repo cannot bring one in.
- Candidates are found by counters. `ToolErrorRate` is a whole-run share
  today, with no per-tool breakdown anywhere, so the per-tool count is part
  of this build.

**Rules kept.** §5 of `SELF-IMPROVEMENT-RESEARCH.md` decides who writes the
text: the owner, or a template over counters. A model never paraphrases an
error string into one (**R3**).

### P6. Lineage is re-checked, and support must be independent — S/M

**Gap.**
- `Rule::sources` records the reflections a rule came from.
- A reflection's `Origin` can be reclassified after the fact
  (`learning::Reflexion::provenance`).
- Nothing propagates that change to a rule built from it.
- Nothing distinguishes a rule supported by three sessions from one
  supported by three interventions in a single session.

**Source.**
- MemLineage: provenance-gated actions drove attack success to zero.
- FARMA: self-reinforcing forged entries defeat consensus checks.

**Shape.**
1. When any source of an active rule stops being learnable, the rule goes
   back on probation, with the reason recorded.
2. A rule whose sources come from a single session is **born on
   probation**. It does not refuse; it gets the shorter leash, matching the
   D1 hedge's idiom ("acting without certainty is the bet, and a shorter
   leash is the hedge"). The independence threshold is **R5**.

**Rules kept.** Narrowing only.

### P7. The reflector is told what never becomes a rule — S

**Gap.** `learning::REFLECTOR_SYSTEM` tells the reflector to skip a
success, a new task, and ordinary conversation. It says nothing about the
three categories that harden into self-imposed constraints:
- a failure caused by the environment (a missing binary, a server that was
  down);
- a negative claim about a tool;
- an approach that was tried and never resolved.

`MISMATCH_REFLECTOR_SYSTEM` already carries one of these ("never derive
stop-work, skip-work, or hard call-limit rules from counts alone"). The
steer, denial and follow-up reflector does not.
`WRITING_REFLECTOR_SYSTEM` (`Trigger::Edit`) is left alone: the three
categories are shaped by tools and environment, and a draft edit is
neither.

**Source.** Hermes's `_DO_NOT_CAPTURE_BLOCK`, written after exactly this
failure in production. PASB's 51.4% shows a bare instruction is not
enough, so the prompt names criteria, not a tone.

**Shape.**
- Three sentences added to `REFLECTOR_SYSTEM`.
- The effect is read as the share of `{"skip": true}` replies, and the
  categories of the reflections that remain, before and after.
  A skip is discarded today (the reply handler drops it and records
  nothing), so recording skips is part of the build.
- The reflector's prompt is not in any run's prefix.

### P8. Behaviour-changing candidates are measured on continued branches — L

**Gap.** `SELF-IMPROVEMENT-RESEARCH.md` §14.6: replay can only measure
changes that change nothing. `HARNESS-RESEARCH.md` §9 recommended moving
config candidates into experiments, and R1 answers it in part (§0). The
branch `fix/selection-slice-power` is unmerged. The deeper gap, found after
R1 was ruled: **every branch instrument mecha has stops at the first call
the recording cannot answer, and grades reproduction of the recording**, so
none can follow a better path. **[`ROLLOUT-DESIGN.md`](ROLLOUT-DESIGN.md) is
the engine that closes it.** Its comparisons (live reads, sunk writes, two
controls, structural scoring) are how P8 is built. The fixture-world arms
below remain for tasks the owner's stores cannot host.

**Source.**
- HarnessLens: paired live rollouts labelled Recovered, Stable or
  Regressed, a fresh confirmation batch, and "preservation alone is not
  enough".
- OpenJarvis: the target cluster improves, and no other cluster falls past
  a tolerance.
- Harness²: re-running the same task is what makes a behavioural contrast
  visible at all.

**Shape.** The primary world is the live dry world: comparisons at
sampled points, built as [`ROLLOUT-DESIGN.md`](ROLLOUT-DESIGN.md)'s phases
(R1). What follows is the **fixture-world variant**, for tasks the owner's
stores cannot host.
- A candidate the replay gate dropped as divergent becomes an `experiment`
  with two arms (current against candidate) in `trial_env` homes.
- **Being free of side effects is a property of the manifest, not of the
  home.**
  - A manifest's `live_servers` can carry the operator's real servers in by
    name.
  - `MACHINE_TABLES` hands every trial the operator's `[[search]]`
    backends.

  So the manifests P8 generates must name no `live_servers`, and only
  fixture stores (`stores/<server>/`) may appear in their environments. A
  check that refuses any other P8 manifest is part of the build. Search
  stays the operator's: it is `Blind` egress, read-only, but its queries do
  leave the machine.
- Tasks are drawn from the episodes whose counters motivated the candidate,
  plus preservation tasks.
- Each pair is labelled Recovered, Stable or Regressed. "Stable" alone never
  accepts.
- The regression ceiling applies per counter, as it does now.
- **The second product is divergence.** For each task, the first tool-call
  index where seeds disagree, and the tool at that point, become a typed
  field in `diagnose::Evidence`. Harness²'s editors rank a divergence first
  ("the agent can already do it"). This hands the diagnostician that signal
  as a number, never as transcript text.

**Is this needed when replay exists?** Yes, for one class of change. Mecha
has three instruments, and each sees a different kind of change:

| Instrument | What it can score | Measured so far |
|---|---|---|
| **Whole-run replay** (the numeric gate) | A change that leaves the agent's *actions* as recorded and alters only their cost: fewer compactions, fewer tokens | 17 candidates and never 8 pairs. The newest diverged 16 times on *each* arm, because the local sampler does not reproduce its own recording |
| **Branch at a recorded point** (`pointwise_pass`, R26/R36; the learn gate's counterfactual) | A change whose effect shows in the **next action** at a moment where a verdict exists | It is why learning works: a steer says what the next action should have been. In the harness gate it has decided **one point in total** (candidate `…bd65`). It needs an owner verdict at the point, and those are scarce |
| **Live paired arms** (this proposal) | A change whose effect shows **many turns later**: `max_turns`, `compact_at_tokens`, a skill, a subagent | Not built |

Replay cannot supply a tool result the recording never had, and a
behaviour change makes exactly those calls. That is structural, not a
tuning problem. So replay stays: as the cheap first screen, and as the
right instrument for cost-only changes. Live arms are needed only for
multi-step effects.

**The cheap middle step, before live arms: verdict-free branch points.**
- The branch instrument is starved because its points need an owner
  verdict. One class of point needs none: **a recorded failed call that the
  agent then repeated identically**, same call and same result, which is
  `compact::target_of`'s test, the one boredom already uses.
- At such a point, the recording already says what the repeat returns.
  Branch there under each arm. The candidate wins the point if its next
  action is not the known-futile repeat, and loses if it is.
- This judges a narrow thing (avoiding a known dead end), but it judges it
  without anyone's verdict, at a supply the corpus already holds. It fits
  exactly the changes Tier 1 aims at first: tool guidance, error hints
  (P5), `effort`.
- It needs no ruling: it is the existing point-wise pass with a second
  point source.

**What R1's yes still leaves open.**
- §14.7 says fitness is the owner's own usage, and fixture worlds are not
  that usage. Re-hosting real recorded tasks in a fixture world is a
  private-data question.
- The local model's throughput bounds how many pairs fit in a night.

### P9. The diagnostician sees failures, not only counts of them — M

**Gap.**
- `diagnose::Evidence` is counters, plus bounded clean `AppraisalNote`s.
  The diagnostician never sees a run.
- `diagnose.rs` treats a decline as "a legitimate answer". But a brief
  that shows only aggregate counters gives it nothing to decline *from*:
  every night the counters are nonzero somewhere.

**Source.**
- Harness²: evidence is a directory of full trajectories, the prompt is a
  pointer to it, and every edit must cite a trajectory moment
  (`core/context.py::mission`).
- AHE: an "agent debugger" turns rollouts into layered evidence per
  failure (`SELF-IMPROVEMENT-RESEARCH.md` §3).

**Shape.** The brief gains an evidence directory, built per episode in the
non-holdout pool:
- **A typed digest of every episode**, all harness-owned and closed-set:
  - the tool-call sequence (names, `is_error`, `external`);
  - the stop cause;
  - the turn at which the owner stepped in;
  - from P8, the first index at which seeds diverge.
- **For a clean session, the transcript itself.** Its provenance is the
  owner and the model: the same class the reflector accepts. The reflector,
  though, sees an intervention excerpt, not a whole transcript, so this
  half is a wider read than anything the loop does today. That is why it is
  a ruling (R6) and not part of the digest.
- **For an armed session, the per-intervention withheld views of that
  episode, concatenated (after R6).**
  - `Intervention::user_evidence_only` reduces one moment: the owner's typed
    text and the tool names before and after it. P9 uses exactly that,
    once per intervention, and nothing between interventions.
  - This is deliberately narrower than a whole-session withheld view. A
    user turn in an armed session can itself hold pasted third-party text.
    That is tolerable over a moment the owner typed in order to intervene,
    and a different bet over every user turn in the session.
  - A whole-session view would be a further widening, and is not
    proposed.

The mission requires every proposal to cite an episode and a turn, and a
proposal that cites none is refused before measurement. The existing
artifact tests still apply: the mandatory prediction, class re-derivation,
and `lifted`. Today `lifted` checks against the tool results and appraisal
notes the diagnostician read; extending its source list to the new evidence
directory is part of this build, or the no-verbatim test silently stops
covering what the diagnostician reads most.

**Rules kept.**
- **P9 crosses §5's letter, and argues it from §5's reason.**
  - `SELF-IMPROVEMENT-RESEARCH.md` §5 says "the input to a prose proposal
    must be a number, never a body of text". Its reason is that "a
    transcript excerpt is third-party text on the longest half-life path".
  - §13.1 relaxed only the *output* form of a web-informed diagnosis. It
    does not license this input.
  - The typed digest is inside §5's letter: closed-set names and counts
    are counters. **It needs no ruling.**
  - The per-intervention withheld views carry the owner's typed words and
    tool names, and **no third-party text except what the owner pasted into
    a steer**. That is the reflector's bet today, taken at the same scale:
    one moment the owner typed in order to intervene. A clean transcript
    carries none by definition. Both are outside §5's letter and, with that
    one bet named, inside its reason.
  - The outputs keep §5's protection where it matters. A config candidate
    is a number, and a prose or architecture candidate stages for the
    owner and never auto-accepts. **Both text halves wait on R6.**
- Most sessions arm early (`PROVENANCE-DESIGN.md`: 15 of 18 on message 1
  or 2), so the clean-transcript half is thin. The digest carries most of
  the weight. Whether the clean half is worth building is **R6**.

**Built (2026-10-19): the digest and the citation rule.**
- **Each digest holds** tool names, the error and external flags, the stop
  cause, turns, and the kind and position of each intervention.
- **The brief shows up to 12 remainder episodes, and never a held-out
  one.** Episodes with a failed call, an intervention or an incomplete run
  come first, and the rest are counted.
- **Citation:** a brief with digests asks for `cites: <episode> #<call>`. An
  uncited proposal is refused before it is measured.
- **Such a brief is untrusted as well as private.** An MCP server names its
  own tools, and a name that passes an identifier check can still be a
  sentence. On those nights, from the first turn:
  - the diagnostician cannot use the fetch tool where it picks the address;
  - its search runs only the blind backends, at quick depth.
  `[agent] digests_in_brief` buys both back.
- **Not built:**
  - The text halves wait on R6.
  - The divergence index waits on P8.
  - `lifted` does not read digests, because there are no words in one to
    lift.

### P10. Structural candidates arrive with a measurement — L

**Gap.**
- `ChangeClass::Architecture` and `Prose` stage with no measurement
  attached.
- The two staged architecture candidates have waited since 2026-09-26 and
  2026-10-11.
- `HANDOFF.md` records that the content-sensitive arm (`mecha eval
  --ab-config`) "is still a human spend", and nobody spends it.

**Source.**
- Harness²'s component table: the gains are in skills, scripts, tool
  guidance and middleware.
- Harmony and OpenJarvis: structural edits behind a held-out gate.
- The inversion this project prefers to a refusal: keep the feature, and
  move the decision to the trusted side.

**Shape.**
- A structural candidate stays a human's call, but the human decides on
  evidence. When the owner marks a staged candidate "worth measuring", the
  owner (or a session working for the owner) implements it behind a lever
  or an arm environment.
- P8 then measures it on paired fixture arms, and the result is written
  into the candidate record before the accept/reject decision.
- The surface this opens, in order of the §3 evidence:
  1. tool descriptions and guidance;
  2. error hints (P5);
  3. subagent profiles;
  4. skills.

  All four are owner-reviewed text. **Skills were the one that touched a
  standing ruling, and R7 settled it** (owner, 2026-10-18):
  - A skill *drafted* by the diagnostician from a measured diagnosis, then
    reviewed and accepted by the owner, is allowed.
  - `SKILLS-RESEARCH.md` §9's refusal of *unreviewed* model-authored skills
    stands. No skill is written into the store by a model.
  - The evidence behind the line: self-generated skills come out ≈0 on
    average (SkillsBench), curated skills give +16.2 pp, and Harness²'s
    editor-drafted, *selected* skills give the largest single component
    effect. Owner review is the selection.
  - A drafted skill is prose plus, at most, references. **An executable
    `scripts/` file is not drafted.** The script-bundling skills in the
    31k-skill audit were about twice as likely to be vulnerable, and a
    script is code, not prose a reviewer reads once.
  - **A drafted skill carries learning's provenance condition.** Its body
    rides every future run's cached prefix, which is the same half-life as a
    learned rule. A learned rule is gated *structurally* on clean
    provenance, with deliberately no knob.
    - So a diagnosis may draft a skill only if everything it read was of
      clean origin: the typed digest, and clean transcripts if R6 admits
      them.
    - A diagnosis that read any armed session's withheld view (§7.3's
      option B) may still propose a config or an architecture change, but
      it does not draft a skill.
    - Owner review stays the selection, not the only gate.
    - This narrows R7 as ruled. It is proposed here so the owner can
      confirm it rather than discover it.

**Rules kept.**
- No lane promotes itself: the owner accepts every structural change.
- The `Security` class is still never measured.
- §6.1's invariants hold before any arm runs.

### P11. Preplay: the same engine, online — L

**The owner's framing:** preplay is replay "just running online in parallel
instead of replayed after the session is over". At a live decision point,
roll out two alternatives a short way in the dry world, score them
structurally, and continue with the better one.

**Shape:** [`ROLLOUT-DESIGN.md`](ROLLOUT-DESIGN.md) §7.
- It runs only at decisions with a structural validator. That is
  `APPRAISAL-WIRING-DESIGN.md` N3's condition, kept.
- It runs on background and delegated runs first.
- It is off by default until it beats matched-compute sampling.
- It never sends.

---

## 6. Deliberately out of scope

- **Model-authored skills, scripts, hooks or plugins.**
  - `SKILLS-RESEARCH.md` §9 stands.
  - SkillsBench (self-generated skills ≈ 0) and the negative-transfer result
    corroborate it.
  - ClawHavoc and the 31k-skill audit show what a writable skill surface
    attracts.
  - Harness²'s best components are exactly these, and the evidence still
    does not move the ruling.
- **Per-task recursion on live runs.** Four executions of a task that sends
  mail is four sends. Harness²'s loop applies only where a run has no side
  effects, which is what P8's fixture worlds are.
- **Third-party text as the input to any proposer.** Harness² and Hermes
  both feed full trajectories, third-party content included, to the editor.
  That is the Zombie Agents path that `SELF-IMPROVEMENT-RESEARCH.md` §5
  exists to close. P9 never shows an armed session's third-party text: it
  shows, after R6, per-intervention withheld views. P9 asks for two
  widenings beyond the typed digest: those withheld views and whole clean
  transcripts, §7.3's options B and C. Both wait on R6.
- **Accepting anything on a judge or a proxy alone.** TTHE and Harness²
  lose at selection, and their editor and judge share a model's biases.
- **"Be active" prompting** of any learner or diagnostician. KEEP_BASE and
  `NO_ENTRY` are first-class answers, and doing nothing is a result.
- **A skill hub or any third-party install path.**
- **Distilling harness gains into weights** (Harness-Zero, ClawGym II, Evo²).
  This is parked, not refused. It fits the local-model premise, but training
  on transcripts needs the provenance gate, and a weight change is far less
  revertible than an override.

### 6.1 Invariants any future widening must keep

If the edit surface ever grows past four knobs and learned rules:
- **The measurement path is out of reach.** No candidate may write to:
  - the validation ledger;
  - the comparison store;
  - the holdout draw;
  - an experiment's manifest or graders.

  Harmony's agent edited its evaluator the moment it could.
- **Graders stay out of a trial home.** Harness² runs a tripwire,
  `assert_no_grader_verdicts`, over every context a model sees, matching by
  file name and by content. Before P8 runs, confirm that no eval case
  check, `mismatch.rs` gold fixture or score file is reachable from a
  `trial_env` home, or through `web_search` from inside one. The concrete
  type to check is `fixture_check::FixtureCheck`: its expected values
  (`equals`, `contains`, `count`) live in the manifest, and its `file`
  resolves inside the trial's own `fixtures/` directory.
  - **The live dry world too.** R1 made it the primary world, so it needs
    the same check. A rollout's `fs_*` tools are jailed to a scratch copy
    of the recorded workspace, unconfined `shell` halts, and the ledgers
    live outside any workspace. So the exposure is the workspace itself:
    a session recorded *in this repository* carries `eval/cases.jsonl` and
    the `mismatch` fixtures into its scratch copy.
  - Before P8 measures a candidate on such a session, its scratch copy
    leaves out the eval and fixture trees, or the session is not offered as
    a point.

---

## 7. Rulings

### 7.1 Ruled, 2026-10-18

- **R1 (P8): yes.** Behaviour-changing harness candidates are measured on
  continued branches. The primary world is the **live dry world** of
  `ROLLOUT-DESIGN.md`: live reads, sunk writes.
  - The owner chose live in so many words: "In a sandbox or live. Not sure
    why we wouldn't want live as every important thing always needs to be
    approved."
  - Fixture-world arms remain only for tasks the owner's stores cannot host.
  - This also answers `HARNESS-RESEARCH.md` §9's recommendation, in part
    (§0).
  - The owner asked whether this is needed given replay. P8's table answers
    it: replay scores cost-only changes, and continued branches score
    multi-step effects.
  - The verdict-free branch points come first, and need no ruling.
  - §14.7's question, where the tasks come from, is answered by the live
    world: they are the owner's own recorded moments. What the live world
    still owes is `ROLLOUT-DESIGN.md` §11's W1–W5.
- **R7 (P10): yes.** An owner-reviewed skill drafted from a measured
  diagnosis is allowed. Unreviewed model-authored skills stay refused, and
  no drafted skill carries an executable script (P10).
  - *Proposed narrowing, awaiting the owner's word:* a skill may be
    drafted only from clean-provenance evidence, learning's condition (P10).
    Review of #712 found that R7 as ruled did not carry it.

### 7.2 Owed
- **R2 (P4).** Should the learner reply in deltas (structural, larger), or
  should convictions be carried by normalised text (narrow, a day's work)?
- **R3 (P5).** Who writes hint text: only the owner, or also counter-templated
  candidates staged through `ruminate` as `Prose` for a person?
- **R4 (P2).** When a re-measured override no longer earns its place, does it
  revert automatically (the direction is toward defaults) or stage for a
  person?
- **R5 (P6).** How many distinct sessions take a new rule off probation:
  two, or more? Rare corrections learn more slowly the higher this is set.

- **R6 (P9).** What may the diagnostician read beyond the typed digest?
  Its options are set out in §7.3.

### 7.3 R6: the options, side by side

Two facts frame this:
- **The diagnostician is the local model.** Every one of the 17 candidates
  records `qwen3.6-35b-a3b`, so nothing below leaves the machine. Its cost
  is context, not privacy: it is a mixture-of-experts model with about 3B
  active parameters.
- **Most sessions arm early.** `PROVENANCE-DESIGN.md` counted 15 of 18 on
  message 1 or 2, so "clean" is a minority.

**A. Typed digest only.** Tool names, `is_error`, `external`, the stop
cause, the turn of each intervention, and seed divergence.
- *For:*
  - inside §5's letter, with no text at all, so attacker-free;
  - covers every session, armed or clean;
  - small enough for a 3B-active model;
  - needs no ruling.
- *Against:*
  - it shows *where* things went wrong and never *what*;
  - the diagnostician still infers the cause, which is how the
    reverse-engineered mechanism in §5.0 happened.

**B. A, plus the per-intervention withheld views.** The owner's typed words
at each steer, denial or follow-up, with tool names before and after.
- *For:*
  - these are the moments where something is *known* to have gone wrong,
    stated in the owner's own words;
  - it is the exact input the reflector already handles every night, in
    the loop that works;
  - it covers armed sessions, which is most of them;
  - the volume is small: 3–6 reflections a week in `learning-report`.
- *Against:*
  - crosses §5's letter;
  - an owner's steer can contain pasted third-party text, which is the
    reflector's exposure today too;
  - the diagnostician and the learner would act on the same signal, so the
    brief must show which rules were already learned from those
    interventions, or both fix the same thing twice;
  - **a different kind of protection from learning's.** Today a non-clean
    reflection is kept from becoming a rule *structurally*, at the learn
    step. Under B, an armed session's withheld view reaches the
    diagnostician, and the protection is the output form only: a number, or
    a prose candidate staged for the owner. That may be the right trade,
    but it is a different trade, and R6 should be decided knowing it.

**C. A, plus clean transcripts in full.**
- *For:*
  - the richest evidence: what Harness² and AHE feed their editors;
  - it shows error text from mecha's own tools, the arguments, and the
    model's reasoning;
  - "clean" is the same fail-closed classification that gates learning.
- *Against:*
  - thin, and **unrepresentative**: sessions that never touched mail, the
    graph or the web are not the owner's typical work, which pulls against
    §14.7;
  - full transcripts are heavy context for a 3B-active model, so it needs
    the Harness² file-and-pointer layout and file tools;
  - crosses §5's letter widest;
  - a single misclassified session would put third-party text on the
    longest half-life path;
  - `lifted` must be extended to cover all of it.

**D. B and C together.** The most evidence, and the sum of both costs.

**Recommendation: B.**
- It adds the highest-value evidence, the moments the owner said were
  wrong, at the lowest new exposure.
- It reuses an input the learning loop has already proven.
- Revisit C only if the digest-plus-B diagnoses are still blind on the
  evidence P1 and P8 produce.

### 7.4 Suggested build order

Tier 1 first, because that is where nothing has worked:
1. P1 and P2: an honest history and a counted noise floor. Both are small,
   and P8's verdicts mean nothing without P2.
2. P9's typed digest: inside §5's letter, so it needs no ruling. It is the
   cheapest change that could stop the blind proposals. The withheld views
   and clean transcripts follow after R6.
3. P8: verdict-free branch points first (no ruling), then continued
   branches, in `ROLLOUT-DESIGN.md`'s phases. Its phase 0 is a defect fix
   that does not wait on anything.
4. P10, after P8, starting with tool descriptions. Skills come last, as
   drafts the owner reviews (R7).
5. Tier 2 alongside, smallest first: P7, P3, P6, then P4 (after R2) and
   P5 (after R3).

---

## 8. Sources

- **Code read:**
  - [`google-research/harness2`](https://github.com/google-research/harness2) at `1cfe254`;
  - [`NousResearch/hermes-agent`](https://github.com/NousResearch/hermes-agent) at `9e01b44d`.
- **Papers:** linked inline in §1–§3.
- **Curated lists used to find them:**
  [DAIR.AI harness-engineering collection](https://academy.dair.ai/papers/collections/harness-engineering)
  and [Awesome-Harness-Self-Improvement](https://github.com/leezythu/Awesome-Harness-Self-Improvement).
- **Secondary coverage** (📰) is used only where marked: ClawHavoc, the
  Socket audit, CVE-2026-9353, the Gemini delayed-invocation and ZombieAgent
  reports, and Hermes's reception.
