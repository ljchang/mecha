# Persona memory: timescales, states, and decay — research

> **Owner rulings, 2026-10-02** (recorded in `PERSONA-DESIGN.md` §9.13 and
> D26–D28; they supersede this report where it differs):
>
> - **Four tiers** (§4.3's table): momentary (episode only), short-term state
>   (days–weeks), ongoing situation (months), lasting fact.
> - **Mood labels about the owner may be stored**, temporarily, as short-term
>   states. This reverses §4.7's "not allowed, even when stated" and its
>   "never `inferred` states" (a mood read from how the owner seems goes to
>   the inferred table and expires), and §9.5's "never a mood label".
> - **Facts are updated as needed** (supersession, §4.2), as built.
> - **A fact derived from several chats is rebuilt** from the chats that
>   remain when one is forgotten (§4.4's open question).
> - **§5.2's tier default is reversed**: a row with no tier, or one this
>   binary cannot read, reads as *lasting* (it over-retains, visibly), not
>   as a state that silently expires. See the note at §5.
> - **Shipped since this pass:** dating from the source chat, in the owner's
>   timezone (#514, `persona::memory::said_at`, `recall::local_day`), so
>   §4.0's second bullet and §4.5's "the fix in progress" describe what is
>   now built. Rendering the *age* and setting `valid_from` are not.
> - **Measure in use and revisit**: every access to a memory is logged with
>   its time, so age-based decay and retrieval frequency can be compared on
>   real use (§1.2's ACT-R form needs the timestamps, not a count) before
>   either is allowed to change recall.
>
> Two claims were re-checked against their primary sources after the pass:
> MemoryBank's `forget_memory.py` at `3869efcc` does compute
> `math.exp(-t / 5*S)`, and PASB's abstract does report 51.4% / 71.9% /
> 45.0%.

Research pass, 2026-10-02. Prompted by the first real night of the persona
memory writer. One persona held 4 episodes and 19 facts, and most "facts
about the owner" were states or one-off events ("driving home", "kids cranky
after daycare", "daughter has [removed]", "[removed]",
"asked for a picture of X"). Durable facts went missing even where they were
implied, such as who the named children are. This builds on
`docs/MEMORY-RESEARCH.md` (curation beats accumulation; invalidate, don't
delete; stale memories are the worst distractors) and does not repeat it.

**Evidence key.** ✅ peer-reviewed · 📄 preprint · 📰 vendor or product doc ·
† cited from the literature without re-fetching the primary source in this
pass (standard results, but not re-checked against the paper today) ·
**unverified**: secondary sources only.

---

## 0. The short answer

The owner's intuition is right, and both cognitive science and the agent
literature back it. What they back, though, is **not** "save everything and
decay it faster". They back three separate moves:

1. **Different kinds of memory have different lifetimes, and that is a
   property of the claim, not of the store.** "Has two daughters" and "[removed]
" are both sentences about the owner. Only one of them describes the
   owner a month from now. Personality psychology has formalised exactly this
   split (state vs trait), and so has the newest agent work: type-conditioned
   decay, where a single uniform decay was shown to fail.
2. **Most of the measured damage comes from time being *absent*, not from
   decay being absent.** Undated memories and memories with no validity
   period are what fail temporal and knowledge-update questions. Systems that
   carry dates and validity, and that render them, are the ones that recover.
3. **Decay is the least-evidenced lever.** Every published decay constant in
   the systems below is a guess that was never ablated on its own. The one
   paper that does ablate decay finds it matters only when it is
   *type-conditioned*, and only on its own benchmark.

So the ranked recommendation (§5) is:

1. date everything from the source chat and **render the age**;
2. give records a **closed timescale class**, where one-off events live only
   in episodes and states carry a coarse expected end (`valid_to`);
3. keep **supersession, not decay, for lasting facts**;
4. treat recurrence as a **consolidation candidate** for the owner, never as
   automatic strengthening;
5. leave decay curves in retrieval scoring as an experiment, not a default.

---

## 1. Cognitive science: what each tradition says about timescales

### 1.1 Episodic vs semantic, and complementary learning systems

Tulving's distinction († Tulving 1972, *Episodic and semantic memory*, in
*Organization of Memory*) separates memory for dated, situated events from
memory for general knowledge. Complementary learning systems theory
(† McClelland, McNaughton & O'Reilly 1995, *Psychological Review* 102:419–457)
explains why the two are kept apart:

- a fast hippocampal system stores individual episodes immediately;
- a slow neocortical system extracts regularities by **interleaved replay over
  many episodes**;
- writing a single episode straight into the slow system causes catastrophic
  interference.

The 2016 update († Kumaran, Hassabis & McClelland, *TiCS* 20:512–534;
† Tse et al. 2007, *Science*) adds a qualification: information consistent
with an existing schema can be assimilated quickly.

**Implication.** A fact (semantic) should be what survives *across* episodes,
not a sentence lifted from one. The first night's failure looks like the
classic one: the writer took single-episode content and committed it to the
slow store. The schema exception also says when quick assimilation is
legitimate. "Has two daughters" fits the owner's existing life structure and
can be written from one mention. "[removed]" is an episode detail.

### 1.2 Forgetting curves: shape, and why it has that shape

- **Ebbinghaus to Rubin & Wenzel.** Rubin & Wenzel fit 105 two-parameter
  functions to 210 published retention data sets (✅ *Psych. Review* 103:
  734–760, 1996; [summary](https://www.semanticscholar.org/paper/2f5d66d8a01e0cab2d7a12821311c5ffc46e4dc6)).
  No single universal function emerged. A small set of power, logarithmic and
  related forms fit best, and a simple exponential did not. Loss slows down
  more than an exponential predicts. Autobiographical memory was the
  exception.
- **Jost's law / Wixted.** Of two memories of equal strength, the older one
  decays more slowly († Wixted 2004, *Annu. Rev. Psychol.* 55:235–269, which
  ties this to consolidation). The decay *rate* itself falls with age.
- **Anderson & Schooler 1991** (✅ *Psych. Science* 2:396–408;
  [PDF](https://users.cs.northwestern.edu/~paritosh/papers/KIP/AndersonSchooler1991ReflectionsOfEnvironmentOnMemory.pdf))
  studied 730 days of *New York Times* headlines, child-directed speech
  (CHILDES) and one person's email. The probability that an item is *needed*
  falls as a **power function of time since last use**, rises with frequency,
  and shows spacing effects. Their claim is that memory's forgetting curve is
  a rational reflection of the environment's need statistics.
- **ACT-R base-level activation** (†, standard form, Anderson & Lebiere 1998;
  confirmed by the [HAI 2025 ACT-R memory paper](https://dl.acm.org/doi/10.1145/3765766.3765803)):
  `B_i = ln(Σ_j t_j^(−d))`, summed over past uses `j`, with `d ≈ 0.5`. Recency
  and frequency combine in one quantity, and decay is a power law.

**Implication.** The right decay for "will this be needed?" is a power law
over *uses*, not an exponential over age. A power law has no single
half-life. It forgets fast early and then flattens, which is the behaviour
the owner described ("facts change very slowly; states are ephemeral") in one
curve. But Anderson & Schooler's need statistics were for *words in an
environment*. Nobody has measured the need-probability of "the owner's
daughter had a fever" in companion chat. The shape transfers; the constants
do not.

### 1.3 Retrieval strengthens: the testing effect and the "new theory of disuse"

Retrieval practice improves long-term retention more than restudy
(✅ Roediger & Karpicke 2006, *Psych. Science* 17:249–255), and spaced
repetitions beat massed ones († Cepeda et al. 2006, *Psych. Bull.* 132:354).

Bjork & Bjork's new theory of disuse (✅ 1992;
[lab summary](https://bjorklab.psych.ucla.edu/research/)) separates two
quantities:

- **storage strength**: how well learned and integrated something is. It
  never decreases.
- **retrieval strength**: how accessible it is now. It decays with disuse.

High storage strength slows the loss of retrieval strength.

**Implication.** This is the cleanest conceptual fit for mecha's append-only
store. Records never lose storage; only their *accessibility* in a given chat
changes. Decay belongs in recall ranking or rendering, never in deletion. The
spacing result also says that a re-mention in a *separate* chat, days later,
is evidence of durability. A re-mention three turns later in the same chat is
not.

### 1.4 Traits vs states: personality psychology's version of the question

This is the closest scientific analogue to the owner's question, and it is
well developed.

- **Lay and scientific criteria for "state" vs "trait"** (✅ Chaplin, John &
  Goldberg 1988, *JPSP* 54:541–557). People classify attributes along
  dimensions that include:
  - duration (states are temporary; traits are long-lasting);
  - consistency across situations;
  - whether the cause is internal or external (states are typically caused by
    circumstances).

  These are exactly the questions a writer prompt could ask of a sentence.
- **Latent state-trait theory** (✅ Steyer, Schmitt & Eid 1999, *Eur. J.
  Pers.* 13:389–408;
  [PDF](https://www.researchgate.net/publication/246848814_Latent_state-trait_theory_and_research_in_personality_and_individual_differences))
  decomposes every measurement into a stable trait part, an occasion-specific
  part, and error. Questionnaire personality scales are mostly trait variance:
  occasion-specific variance has a median of about 7% across inventories.
  Mood and affect measures carry much more occasion variance.
- **Traits as density distributions of states** (✅ Fleeson 2001, *JPSP*
  80:1011–1027; [PDF](http://simine.com/407/readings/Fleeson_2001.pdf)).
  Experience sampling over 2–3 weeks found that within-person variability is
  so large that "the typical individual regularly and routinely manifested
  nearly all levels of all traits". Yet each person's *mean* of the
  distribution was almost perfectly stable. **A trait is a summary of many
  states**, not a different kind of observation (see also Fleeson &
  Jayawickreme 2025,
  [*Eur. J. Pers.*](https://journals.sagepub.com/doi/10.1177/08902070251366709)).
- **How slow "slow" is.** The test-retest rank-order consistency of traits
  rises from .31 in childhood to .54 in college, .64 at 30, and plateaus
  around .74 between 50 and 70, at a constant 6.7-year interval (✅ Roberts &
  DelVecchio 2000, *Psych. Bull.* 126:3–25;
  [PDF](http://jenni.uchicago.edu/Spencer_Conference/Representative%20Papers/Roberts%20&%20DelVecchio,%202000.pdf)).
  Traits are slow, but they do change over years.
- **In text, the balance flips.** On 5,001 contextual profiles from 1,667
  Reddit users, 72–74% of psychological variance in what people *write* was
  within-person (state) and only 26–28% between-person (trait). LLMs given a
  profile were "state-blind": they responded to trait and ignored state
  (📄 Harry et al. 2026, *State Beats Trait*,
  [arXiv:2601.15395](https://arxiv.org/abs/2601.15395)). Some of that
  variance is measurement noise, though the authors argue against noise as
  the whole explanation.

**Implication.**

1. Most of what an owner says in a chat is state. So a writer that records
   "what was said about the owner" will mostly record states, which is
   exactly what happened on the first night. The base rate predicts the
   failure.
2. A trait should be **derived from repeated states**, not extracted from
   one. That is Fleeson's model, and it is CLS consolidation by another name.
3. The usable criteria for the writer are Chaplin et al.'s: *expected
   duration*, *situational cause*, *would it be true in a month?*

### 1.5 Event segmentation, situation models, and the missing middle tier

- **Event segmentation theory** († Zacks et al. 2007, *Psych. Bull.* 133:273)
  holds that people segment experience at points where prediction error
  rises. **Situation models** († Zwaan & Radvansky 1998, *Psych. Bull.*
  123:162) index events by time, space, causality, intention and
  protagonist. Crossing an event boundary makes the prior situation's details
  less accessible († Radvansky's event-horizon model, 2012).
- **Autobiographical memory is hierarchical** (✅ Conway & Pleydell-Pearce
  2000, *Psych. Review* 107:261–288;
  [PDF](https://www.researchgate.net/publication/12528554_The_Construction_of_Autobiographical_Memories_in_the_Self-Memory_System)).
  It has three levels:
  - **lifetime periods** ("when the kids were in daycare");
  - **general events** ("the week the youngest was sick");
  - **event-specific knowledge** ("she had a fever on day six").

**Implication.** A state is not free-floating. It belongs to a *situation*,
and it ends when the situation ends. The episode is the natural container for
it. Conway also names a tier between "fact" and "state" that a two-way split
misses: the **lifetime period**, which lasts months and is true now but not
forever ("works nights this term", "is renovating the kitchen", "the kids are
in daycare"). A design with only "permanent" and "ephemeral" will force these
into the wrong bin.

### 1.6 Summary across traditions

| Tradition | Slow (fact/trait) | Fast (state/event) | Mechanism of change |
|---|---|---|---|
| CLS / Tulving | semantic, built by interleaving many episodes | episodic, stored at once | consolidation; schema-consistent = fast |
| Forgetting curves | low decay rate (Jost) | steep early loss | power law, decay slows with age |
| Rational analysis / ACT-R | high frequency × recency | one use, long ago | `ln Σ t^(−0.5)`: need-probability |
| New theory of disuse | high storage strength | high retrieval, low storage | access decays, storage never does |
| Trait/state (LST, Fleeson) | the mean of the state distribution | most observed variance | traits are aggregates of states |
| Conway / event models | lifetime periods, self-knowledge | event-specific knowledge | situations bound states; boundaries end them |

---

## 2. AI agent and companion memory systems: the concrete mechanisms

Half-lives below are computed from the published constants. Several papers
give no unit, and that is noted where it applies.

| System | Timescale mechanism | Parameters (as published) | Validity / supersession |
|---|---|---|---|
| **Generative Agents** (✅ Park et al., UIST 2023, [arXiv:2304.03442](https://arxiv.org/abs/2304.03442)) | score = α_rec·recency + α_imp·importance + α_rel·relevance, each min-max normalised to [0,1], all α = 1. Recency is exponential decay over game hours **since last retrieved**. Importance is a 1–10 LLM "poignancy" rating set at write time. Reflections fire when summed importance of recent events exceeds 150 (about 2–3 per game day). | decay factor 0.995/game hour → half-life ≈ 138 game hours (≈ 5.8 game days), reset on every retrieval | none; reflections sit alongside observations. The ablation removes whole components (full 29.89 vs no-reflection 26.88 TrueSkill μ); **recency is never ablated on its own** |
| **MemoryBank / SiliconFriend** (📄 Zhong et al., AAAI 2024, [arXiv:2305.10250](https://arxiv.org/abs/2305.10250)) | Ebbinghaus `R = e^(−t/S)`. S starts at 1. On recall, S += 1 and t resets to 0. Also daily summaries and a global "user portrait". | t in days (code). The paper calls it "exploratory and highly simplified". | none. Forgetting is deletion: [the reference code](https://github.com/zhongwanjun/MemoryBank-SiliconFriend/blob/3869efcc/memory_bank/memory_retrieval/forget_memory.py) drops a memory when `random() > R`. That code computes `math.exp(-t / 5*S)`, which Python evaluates as `exp(−(t/5)·S)`: **higher strength forgets faster**, the inverse of the paper's formula. **No ablation of forgetting.** |
| **MemGPT / Letta** (📄 Packer et al., [arXiv:2310.08560](https://arxiv.org/abs/2310.08560)) | Tiers by *location*, not time: in-context working memory / core blocks, a FIFO message queue with recursive summary on eviction, recall storage (all messages), archival storage (vector). | none time-based | the model self-edits core memory; no validity periods |
| **Mem0 / Mem0g** (📄 Chhikara et al., [arXiv:2504.19413](https://arxiv.org/abs/2504.19413)) | Extracts candidate facts, then an LLM picks ADD/UPDATE/DELETE/NOOP against similar memories. Mem0g adds a graph whose conflict resolver marks relations **invalid rather than removing them**. | no decay | base Mem0 deletes; Mem0g invalidates |
| **Zep / Graphiti** (📄 Rasmussen et al., [arXiv:2501.13956](https://arxiv.org/abs/2501.13956)) | Bi-temporal edges: `t_valid` and `t_invalid` (world time), plus created and expired (system time). The extraction prompt resolves relative dates against each message's reference timestamp: "If the fact is written in the present tense, use the Reference Timestamp for the valid_at date". New edges invalidate overlapping contradicting edges. | no decay | context is rendered as `FACT (Date range: from - to)` |
| **A-MEM** (📄 Xu et al., [arXiv:2502.12110](https://arxiv.org/abs/2502.12110)) | Zettelkasten notes with a timestamp. New notes trigger "memory evolution", which rewrites linked notes' context and tags. | no decay | rewriting in place |
| **MemoryOS** (📄 Kang et al., EMNLP 2025, [arXiv:2506.06326](https://arxiv.org/abs/2506.06326)) | Short-term FIFO of dialogue pages, then mid-term topic segments, then long-term persona. Heat = α·N_visit + β·L_interaction + γ·R_recency, with `R_recency = exp(−Δt/μ)`. Segments with heat > τ promote to long-term persona memory (a 90-dimension "User Traits" profile plus a 100-entry FIFO user knowledge base). The lowest heat is evicted. | μ = 1e7 s → half-life ≈ 80 days; τ = 5 | traits are LLM-updated; KB entries age out FIFO. Ablations remove tiers, not the decay term. |
| **LD-Agent / "Hello Again!"** (📄 Li et al., NAACL 2025, [arXiv:2406.05925](https://arxiv.org/abs/2406.05925)) | Long-term *event* memory of session summaries. Score `λ_t·(s_sem + s_top)` with `λ_t = e^(−t/τ)`; semantic floor γ = 0.5. Short-term cache is flushed into an event after 600 s idle. **Separate dynamic user and agent persona memory**, extracted per utterance. | τ = 1e7 (unit unstated; ≈ 80 days if seconds) | none. Ablation: event memory contributed most (BLEU-2 5.48 → 7.57; user persona → 7.54; full 10.70). **Decay is not ablated.** |
| **CoALA** (📄 Sumers et al., TMLR 2024, [arXiv:2309.02427](https://arxiv.org/abs/2309.02427)) | Taxonomy: working memory; long-term episodic, semantic, procedural. From Soar, whose memories use ACT-R-style base-level activation and BLA-based forgetting (✅ Derbinsky & Laird 2013, *Cog. Sys. Res.* 24:104–113). | — | — |
| **ScrubJay-MEM** (📄 Bhandari et al. 2026, [arXiv:2608.04746](https://arxiv.org/abs/2608.04746)) | Each memory gets an LLM-classified class {stable, procedural, task, ephemeral}, a perishability π ∈ (0,1] and a utility horizon τ. Utility = `V·exp(−π·(t_q − t_i)/τ)`. A keyword fallback maps "today/immediate" to ephemeral, π = 0.9. Classes are revised retroactively. | stable π ≈ 0.05, ephemeral ≈ 0.9; horizons from minutes–hours (ephemeral) to days–weeks and longer | the authors' own caution: a mislabelled "ephemeral" medical or safety fact is silently forgotten, so treat π as advisory and gate deletion behind policy |
| **ACT-R-inspired LLM memory** (✅ HAI 2025, [ACM](https://dl.acm.org/doi/10.1145/3765766.3765803)) | base-level activation with d ≈ 0.5 over retrievals, plus semantic match | d = 0.5 | — |

**Companion and assistant products** (📰; none publishes a decay rule):

- **Kindroid** ([help centre](https://kindroid.ai/docs/article/memory/)) has
  three groups:
  - *persistent* memory: backstory and **key memories**, user-written and
    always in context;
  - *cascaded* memory: a medium-term tier that recalls "exceptional
    experiences and recent events more vividly" and resets at a chat break;
  - *retrievable* memory: AI-consolidated long-term memory, surfaced by
    "relevance, recency, and diversity", plus user-written **journals**
    (≤500 entries, ≤3 recalled per message, matched only on the *user's*
    words).

  The split is by author and by tier. Durable things are something the user
  writes. Recency is a retrieval signal, not a deletion rule.
- **Nomi** ([Mind Map 2.0](https://nomi.ai/updates/mind-map-2-0-bringing-nomi-memory-into-view/))
  has short, medium and long-term memory. Its Mind Map holds user-editable
  overviews of "people, places, topics, and goals", designed to "assist
  actual memories, not replace or alter them". Its page says nothing about
  current vs past state.
- **ChatGPT.** It automatically manages saved memories ("no more 'memory
  full'"), and memories can be sorted by recency and re-prioritised
  ([OpenAI, 2025-10-15](https://x.com/OpenAI/status/1978608684088643709)).
  The claim that it weighs "how recent a detail is and how often you talk
  about a topic" is **unverified**: it comes from secondary write-ups of the
  release notes, because the help centre refused fetches. That would be
  ACT-R's recency × frequency, applied as *backgrounding*, not deletion.
- **Claude.** Memory is saved as "a set of individual topics as you chat",
  scoped per project, with personal and sensitive topics excluded by default
  ([support](https://support.claude.com/en/articles/11817273-use-claude-s-chat-search-and-memory-to-build-on-previous-context)).
  An older "synthesis updated every 24 hours" description appears in
  secondary sources (**unverified** against the current page).
- **Character.AI** has user-pinned memories and a short chat-memory box
  (**unverified**: limits come from secondary sources). **Replika** was not
  researched in this pass.

**Pattern.** Production companions answer the timescale question **by
author, not by decay**. Durable things are user-written and pinned (key
memories, pins, about-me). The model's consolidated memory is recalled by a
relevance-and-recency mix. No product found states a TTL for states.

---

## 3. Benchmarks and evidence: what fails, and does decay help?

### 3.1 What fails is time and updates, consistently

- **Missing timestamps are catastrophic for temporal questions.** On LoCoMo,
  OpenAI's memory scored "below 15%" on temporal questions, "primarily due to
  missing timestamps in most generated memories despite explicit prompting",
  while Mem0g reached J = 58.13 (📄 Mem0 paper §4). This is the vendor's own
  comparison, but the mechanism is the same as mecha's dating bug.
- **LongMemEval** (✅ ICLR 2025, [arXiv:2410.10813](https://arxiv.org/abs/2410.10813)):
  - "Naive time-agnostic memory designs perform poorly on temporal
    reasoning". Indexing facts with timestamps plus time-aware query
    expansion improved temporal recall by **6.8–11.3%**.
  - Expanding keys with extracted user facts raised recall@k by 9.4% and QA
    by 5.4%.
  - Compressing sessions *into* user facts lost information overall.
  - ChatGPT's memory "tended to overwrite crucial information as the chat
    continues".
- **LoCoMo** (✅ ACL 2024, [arXiv:2402.17753](https://arxiv.org/abs/2402.17753)):
  LLMs lag humans by 56% overall, and **by 73% on temporal reasoning**
  (the paper's §1 findings list; the abstract states the temporal weakness
  only qualitatively).
- **Knowledge updates and selective forgetting are the weak spots.**
  - MemoryAgentBench (✅ ICLR 2026, [arXiv:2507.05257](https://arxiv.org/abs/2507.05257)):
    every method reached **at most 28%** on multi-hop fact consolidation, and
    only long-context agents did reasonably on single-hop.
  - HorizonBench (📄 2026, [arXiv:2604.17283](https://arxiv.org/abs/2604.17283)):
    6-month histories averaging about 4,300 turns. The best of 25 models
    reached 52.8%. When models erred on an evolved preference, **over a third
    of the time they chose the user's originally stated value**: the stale
    state, unrevised.
  - Memora/FAMA (✅ Findings ACL 2026, [arXiv:2604.20006](https://arxiv.org/abs/2604.20006)):
    "frequent reuse of invalid memories"; memory agents give "marginal
    improvements".
  - PersonaMem (✅ COLM 2025, [arXiv:2504.14225](https://arxiv.org/abs/2504.14225)):
    frontier models reach about 50% on tracking evolving profiles.
- **Production failures are mostly forgetting failures, not recall
  failures.** A mutation-time LLM hook lifted adversarial forgetting cases
  from about 70% to about 93% (📄 [arXiv:2606.15903](https://arxiv.org/abs/2606.15903)).
- **Bi-temporal validity helps, though not uniformly.** On LongMemEval, Zep
  reports temporal reasoning 36.5 → 54.1% (gpt-4o-mini) and 45.1 → 62.4%
  (gpt-4o) over full context. Knowledge-update moved −3.4% and +6.5%.
  These are vendor numbers.

### 3.2 Writers mis-type claims: the first night, measured elsewhere

PASB (📄 2026, [arXiv:2607.10526](https://arxiv.org/abs/2607.10526)) tested
1,600 tasks on two real self-improving agents (Hermes-Agent, OpenClaw) across
12 models:

- "agents save [a user's claim] as a stable preference, a background fact, or
  a reusable procedure in **51.4%** of runs";
- downstream failure was 71.9% when a later chat could read the saved claim,
  against 45.0% when it could not.

That is the state-to-trait inflation seen in mecha's store, and it is shown
to be harmful downstream. The writer's prompt already says "one short
sentence that will stay true" (`persona::writer::SYSTEM`), and the model
still wrote states. A bare instruction is not enough. The prompt needs
criteria and a place for the state to go.

### 3.3 Stored profiles amplify agreement

- User memory profiles gave the largest sycophancy increases, for example
  +45% for Gemini 2.5 Pro (✅ Jain et al., CHI 2026,
  [arXiv:2509.12517](https://arxiv.org/abs/2509.12517); already §2.5 of the
  design).
- PersistBench (📄 [arXiv:2602.01146](https://arxiv.org/abs/2602.01146)) found
  a median **97%** failure on memory-induced sycophancy samples and 53% on
  cross-domain leakage across 18 LLMs.
- MemSyco-Bench (📄 [arXiv:2607.01071](https://arxiv.org/abs/2607.01071))
  reports that existing memory systems "often increase sycophancy". It
  separately tests whether an agent can track memory updates and respect a
  memory's scope.

### 3.4 Does decay or recency weighting itself help? The evidence is thin

- **No paper among Generative Agents, MemoryBank, MemoryOS or LD-Agent
  ablates its decay term alone.** Their constants (0.995/hour; μ = τ = 1e7)
  are unjustified choices. MemoryBank's reference implementation inverts its
  own formula, and nobody appears to have noticed. That suggests the decay
  term was not doing measurable work.
- **The one decay ablation found:** ScrubJay-MEM's *type-conditioned* decay.
  Removing it cut its generalisation gap metric 5.7× (+0.108 → +0.019). Its
  authors also state that "naive recency penalizes durable memories along
  with ephemeral ones". But:
  - the benchmark (TGT: 20 instances) is the authors' own and is built on the
    same four perishability classes;
  - gains "narrow under stronger backbones and reverse on fact-consolidation
    tasks";
  - the +2.66 F1 over Mem0 on MemoryAgentBench EventQA is small.
- **The strongest effects in §3.1 come from time being represented**
  (timestamps, validity, time-aware filtering, supersession), not from
  scoring decay.

**Plain statement.** There is good evidence that *undated, unversioned*
memory fails. There is weak, single-source evidence that *type-conditioned*
decay helps. There is no evidence, either way, on decay constants for a
companion, and no evidence from human-rated companion outcomes at all.

---

## 4. Design options mapped to mecha

### 4.0 Two observations about the code as it stands (not an audit)

- `Memory::recall_search` adds recency as an **RRF rank**. A rank is
  scale-free. If nothing newer exists, a three-month-old state still ranks
  "most recent", and a 2-day and a 2-month gap with the same order score
  identically. "Weighted toward recent" cannot express "stale" today.
- Facts take their recency date from `ingested_at`, which is the dating bug
  being fixed. The writer always passes `valid_from: None, valid_to: None`,
  so the bi-temporal columns exist but carry nothing.

### 4.1 The options

**(a) A three-way taxonomy: lasting fact, state, event.**

- *For:* this is Chaplin/LST/Conway in schema form. It matches ScrubJay's
  finding that one decay rate is wrong. Recall and rendering can treat each
  class differently.
- *Against:* it is a classifier the writer can get wrong (ScrubJay's
  mislabelled-medical caution), and "event" duplicates what episodes already
  hold.
- *Fit:* it needs a new closed enum column, and a closed enum on an
  append-only store is a wire format.

**(b) One fact table with a writer-estimated half-life or duration.**

- *For:* this is ScrubJay's continuous π/τ, and it handles Conway's middle
  tier.
- *Against:*
  - a free numeric estimate from a local model is a model-authored score
    with false precision;
  - LLM duration commonsense is a known weak spot, though only partially
    checked here: duration questions were the hardest category in a 2025
    robustness study ([arXiv:2503.17073](https://arxiv.org/abs/2503.17073),
    **unverified**, search snippet only);
  - two writes of the same fact will disagree.
- *Better:* a **closed bucket** (hours / days–weeks / months / lasting) that
  sets `valid_to` coarsely. That is (b) collapsed into (a).

**(c) States folded into episodes only.**

- *For:* the episode already has a date, a situation and a source. Conway and
  event segmentation say states live inside situations. Nothing new to build.
  Privacy-positive: a child's fever never becomes a standalone record about
  the owner.
- *Against:*
  - an ongoing state that matters across chats ("in the middle of a grant
    resubmission", "recovering from surgery") is buried in prose;
  - per-turn recall may not surface it;
  - a persona that does not know the owner is unwell can be tone-deaf.
- *Right for:* momentary and one-off things ("driving home", "asked for a
  picture").

**(d) Where decay acts.** These are three different levers.

- **Rendering** (show the age: "as of 30 Sep, 2 days ago") is cheap and the
  best evidenced: Zep's date ranges, LongMemEval's timestamps, Mem0's <15%
  without them. It lets the model reason ("six days of fever as of Tuesday;
  ask how she is"). It never hides anything.
- **Retrieval scoring** (age-aware weight) means replacing the rank with a
  function of actual age. A power law, per §1.2, would be better than an
  exponential. *For:* it fixes the scale-free rank. *Against:* the constants
  are unevidenced, and decay that suppresses lasting facts is the failure
  ScrubJay names. So apply it per class, and not at all to lasting facts.
- **Deletion/expiry** is incompatible with append-only plus `source` as the
  deletion key, and it is MemoryBank's design. The append-only analogue is
  **validity**: past `valid_to`, a record is not *recalled* at chat start but
  stays on the curation page and is reachable by `memory_search`, labelled as
  past. That is the new theory of disuse: storage stays, retrieval drops.

**(e) Reinforcement on re-mention.**

There are two different things here.

- **Owner re-mentions across separate chats** are spaced evidence of
  durability (§1.3) and the raw material of Fleeson's trait-as-distribution.
  They should feed **consolidation** and nothing else.
- **Retrieval-driven strengthening**, as in Generative Agents' recency reset
  and MemoryBank's S += 1, means the *persona's own recalls* make a memory
  more recallable. That is rich-get-richer: fixation by construction. It also
  lets the model steer its own memory, which is what §9.7's "the owner's
  words key the search" rule exists to prevent.

  REALM (📄 [arXiv:2609.16053](https://arxiv.org/abs/2609.16053)) reports
  gains from retrieval-driven reconsolidation on LoCoMo and LongMemEval, but
  those are QA benchmarks with no fixation or sycophancy measure.

### 4.2 Decay vs supersession

They answer different questions and should not be merged.

- **Supersession** answers "is this still true?". It needs *evidence*: a
  later statement, or the owner's correction. It is the right tool for
  lasting facts (Zep, Mem0g, mecha's update = invalidate + add).
- **Decay or expiry** answers "should I assume it is still true without new
  evidence?". It is a *prior*, and the right tool for states, which are
  usually never explicitly retracted. Nobody says "my back stopped hurting".
- A state needs both. It expires on its prior, and it is superseded early if
  the owner says otherwise.

### 4.3 Plausible timescales

These are judgement, not measurement. No study supplies them.

| Class | Examples (generic) | Store as | Default validity | Chat-start | Per-turn recall |
|---|---|---|---|---|---|
| momentary | on the train home, the dog restless, asked for a picture | episode prose only | — | via episode | via episode |
| acute state | child [removed], stiff shoulder, travelling this week, deadline Friday | state record, `valid_to` = source date + bucket (days / ~2 weeks) | days–2 weeks | only while valid, with age | while valid; after, labelled "past" |
| ongoing situation (Conway's lifetime period) | new job, kitchen renovation, a child in daycare, a term of night shifts | state record, `months` bucket | ~1–6 months; re-confirm | while valid, with age | yes |
| lasting fact | has two daughters (named), allergy, partner's name, values, long-held preferences | fact | none: supersession only | yes, pinned/recent first | yes |
| episodes | what a chat was about | episode | — | last few | age-aware ranking; never expired |

Reference points:

- Generative Agents' recency half-life is about 5.8 game days, refreshed on
  use.
- MemoryOS and LD-Agent use about 80 days.

If an age-aware episode weight is tried, a power law with ACT-R's d = 0.5
over days since the episode is a defensible starting point. It is an
experiment (§13), not a default.

### 4.4 How a state becomes a fact (consolidation)

Follow Fleeson and CLS. The writer, or the nightly consolidation of §9.12,
proposes a lasting fact when the **same kind of state recurs across ≥ N
separate chats spread over ≥ M weeks**. Example: back pain mentioned in four
chats over two months becomes "has recurring back pain".

- The derived fact cites the state records behind it.
- It enters as a **candidate** the owner accepts, as persona self-updates do
  under D4, because "recurring back pain" is a claim about the owner that no
  single chat stated.
- Open question for the owner: with `source` mandatory and single-valued, a
  multi-source derived fact needs a ruling on what `forget` of one
  contributing chat does to it. Delete it, or re-derive it from what remains?

The other direction matters as much. **Durable facts that states imply should
be extracted from the first mention.** "My daughter [removed]", when the
owner has named her, implies a lasting family fact. This is schema-consistent
fast learning (§1.1) and LongMemEval's fact-augmented keys. A writer prompt
should ask explicitly: *for every person named, is there a lasting fact
saying who they are to the owner?*

### 4.5 Showing a stale state honestly

- Render states in the **past tense with the source date and age**, and
  lasting facts in the present tense. For example: "As of Tue 30 Sep (2 days
  ago), the owner said their daughter had [removed]."
- **Never re-tense a state into the present at render time.** The writer
  stores the sentence as said, dated from the chat (the fix in progress).
- An expired state does not ride at chat start. If recalled per turn or by
  `memory_search`, it is labelled "past (as of …)".
- Add one standing line beside "facts about the owner are context, not a
  reason to agree": *states are as of their date; if one matters, ask rather
  than assume*. Whether that line holds should be measured, like the
  sycophancy line (§9.5).

### 4.6 Companion-specific risks

- **Sycophancy.** Stored profiles amplify agreement (§3.3). States are
  *more* profile-like in effect than facts, because "is stressed about X"
  invites validation. Keeping states short-lived and dated limits how long
  they shape tone. Keeping `inferred` states out entirely (below) removes the
  worst case.
- **Fixation.** A persona that opens every chat with "how's your back?" for
  three weeks is the companion version of experience-following
  (MEMORY-RESEARCH §1). Three things prevent it:
  - expiry from chat-start;
  - no retrieval-driven strengthening;
  - a per-chat cap on how many states may ride.
- **Privacy.** States are disproportionately health, family and location
  (a child's illness; "driving home"). Shorter recall lifetimes reduce
  exposure but do not erase anything. Only `forget` erases, which is correct
  but should be said on the curation page. ChatGPT and Claude both say they
  avoid or exclude sensitive and health details by default. mecha's
  owner-curated, local design differs on purpose, but it is a reason not to
  turn momentary health asides into standalone records.

### 4.7 The tension with §9.5 ("Never: … a mood label …")

A "state" category sits right next to that rule. The line that keeps them
compatible:

- **Allowed:** *stated circumstances*. Health events, logistics, situations,
  plans, things that happened. "Daughter [removed]"; "[removed]";
  "travelling to a conference this week".
- **Not allowed, even when stated:** an *affect label as a record*. "Owner is
  stressed", "owner is sad tonight". Feelings stay in the episode prose,
  dated and in context. They are never extracted into a typed row the persona
  can recall as a property of the owner, because that is a mood label with
  extra steps.
- **Never `inferred` states.** An inferred state ("seems exhausted") is a mood
  label by another name. Inferred records, already apart under D18, should be
  limited to lasting-class claims.
- **The writer estimates a state's *expected duration*, not the owner's
  condition.** A coarse closed bucket about the circumstance is not a score of
  the owner. A numeric "intensity" or "importance" (Generative Agents'
  poignancy) *would* be, and should not be adopted.

---

## 5. Recommendation, ranked

1. **Date from the source and render the age**: (d) at rendering. Required
   whatever else is chosen, and the best-evidenced item in this report: Mem0's
   <15% temporal score without timestamps, LongMemEval's +6.8–11.3%, Zep's
   date ranges. Also set `valid_from` from the source chat. Today it is
   always `None`.
2. **Hybrid of (a) and (c): momentary things go to episodes only; states get
   a closed timescale bucket that sets `valid_to`; lasting facts never
   decay.**
   - Add a closed enum `lasting | state` on facts, with a `state` duration
     bucket (`days | weeks | months`). *(Corrected on review: an unknown or
     missing tier reads as `lasting`, not `state`. Degrading to `state`
     would let a durable fact written by a newer binary silently stop
     riding when an older one reads it — ScrubJay's mislabelled-medical
     caution, §2 — and would leave its `valid_to` undefined. Over-retention
     is visible on the curation page; silent expiry is not. The tier is
     therefore best stored as an `Option`, with none rendered as lasting
     until a writer classifies it.)*
   - Expiry is a *recall* rule, never deletion: past `valid_to`, a state is
     not at chat start and is labelled "past" when searched.
   - The writer prompt gets Chaplin et al.'s three questions (would it be true
     in a month? caused by circumstance? momentary?), the "who is every named
     person" check, and an explicit home for each class.

   This is the option the trait/state and event-model literature supports
   most directly, and the PASB result shows why the prompt must give states
   somewhere to go.
3. **Supersession for lasting facts, as built.** No change. Do not add decay
   to them; ScrubJay and Jost's law both say uniform decay is the wrong shape.
4. **(e) as consolidation only**: recurring states across separate, spaced
   chats become *candidate* lasting facts for owner acceptance (§9.12 path).
   No retrieval-driven strengthening, because of fixation and
   model-steered memory.
5. **(d) at retrieval scoring, as an experiment.** Replace the scale-free
   recency rank with an age-aware weight for episodes and live states only.
   Run it under §13 with three probes:
   - a staleness probe: plant a state, advance the clock, check the persona
     does not assert it as current;
   - an unprompted-mention rate for expired states (fixation);
   - recall of lasting facts after many chats, to catch any harm.
6. **(b) a free numeric half-life**: not recommended. It is a model-authored
   score with false precision. Use (2)'s closed buckets instead.
7. **Decay by deletion** (MemoryBank style): rejected. It is incompatible
   with append-only storage and with `source` as the deletion key, and its
   reference implementation shows how easily it goes wrong unnoticed.

**What would change this ranking:** a measured result (§13) showing that
chat-start states, even dated, raise agreement on planted false claims or the
fixation rate. That would push states toward option (c) entirely, recalled
only per turn and never at chat start.
