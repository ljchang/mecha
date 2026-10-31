# Persona lives — design

> **Status (2026-10-31):** designed, nothing built. Every decision in §16 is
> **open** except PL23 and PL27, which carry the owner's direction of the same
> day (no approval queue for news). §1 is the owner's requests, in the order they were made; §2 is
> the evidence, researched the same day, with a source on every figure that
> carries a decision. Personas, their chats and real people appear nowhere in
> this document: every example is invented.
> A persona is "she" here only because one pronoun reads more easily than
> "it"; nothing in the design depends on it.

**2026-10-31.** One question: *how does a persona come to have something new to
say — a life of its own, a world it follows, people in it, and a model of the
owner's people — without that life costing the guarantees `PERSONA-DESIGN.md`
already holds?*

The symptom that started it: over many sessions a persona repeats itself, and
falls back on memories of past conversations without ever bringing anything
new. `PERSONA-DESIGN.md` §12.7 treated repetition as a sampler problem and
fixed what a sampler can fix (no seed, earlier reasoning dropped). The shape
repetition it left — the same openers, the same topics — is a *content*
problem: a persona has two inputs, what the owner says and what its memory
recalls, and with nothing else to draw on it can only recycle. This design
gives it a third input.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| What is new? | A **world** per persona: a simulated day, events in it, the people in it, and the parts of the real world it follows — beside, never inside, its memory | §3 |
| How is her day made? | An owner-approved weekly routine, filled in by a sampler fitted to American Time Use Survey diaries. Code, seeded, no model | §4 |
| Where do events come from? | A deck of small authored events with preconditions, drawn by a pacing director with a rhythm: a dense floor of minor events, rare major ones, a quiet spell after a peak | §4.3 |
| Does a model write her life? | Only the last step: it narrates the few events the code chose. Code decides what happened and when | §4.4 |
| What does she remember? | **Only what was discussed.** An event she never mentioned stays in the world until it decays, then is gone. One she told becomes a memory with a pointer to the world | §3, §5 |
| Does the world grow forever? | No. Events decay by importance and age and are pruned; *state* (a job, a relationship) persists until superseded | §5, §9 |
| Who is in her life? | A network of her own: people the owner writes, people she mentions, people the simulation needs. Edges carry their history, dated | §6 |
| Does she know my people? | A separate network, learned **only** from what the owner tells her. An unknown name is a gap she may ask about — at most once a chat | §7 |
| Does she follow the news? | Through feeds the owner registers, sorted into three shelves by how much a value can carry: parsed facts, typed gists, and prose read on demand | §8 |
| Can news poison her memory? | Not by default: prose never enters memory or the world as text, and an article chat is remembered as a typed episode with no approval queue. A per-persona switch remembers article chats in full, at a stated cost | §8.4 |
| Pictures? | A camera roll rendered overnight from moments the simulation found photo-worthy, shared instantly in chat | §10 |
| Social media? | An in-app feed only. Never a real account | §11 |
| Does she message first? | Not in the first build. If ever: off by default, capped, gated by mood, never at fixed times | §12 |
| How do we know it works? | `persona::echo` already measures repetition; an initiative reading measures who brings new things; a three-part gate (schema conformance, a stated bit ceiling, a steering arm) decides each news shelf, the parsed-facts shelf before it reaches a prompt and the typed-gist shelf before it is built | §14 |

---

## 1. The owner's requests (2026-10-30 and 2026-10-31)

In the order they were made, paraphrased closely. Each is answered by a
section; none is ruled yet (§16).

- **O1 — Something new.** Personas get repetitive and only ever return to past
  conversations. Real friends talk about what happened to them.
- **O2 — A simulated day.** A light simulator of the persona's day, drawing on
  time-use research: sleep, breakfast, class, lunch, practice, dinner, a club
  meeting, homework in the library with friends. Stochastic, with state
  transitions.
- **O3 — Things happen.** Random events, something surprising at work or at a
  regular event — emotionally notable events that meant something to her.
- **O4 — Her people.** Friends, children, partners, professors, colleagues:
  who she interacts with.
- **O5 — Pictures.** A selfie, a picture of the game, photos from a party.
- **O6 — Social media.** Could she have accounts she manages?
- **O7 — Current events and interests.** She is aware of current events, knows
  more about her own interests and hobbies, and follows news related to them.
- **O8 — A world model that decays.** A per-persona world that ingests her
  world and her view of it; details decay so it never grows too big.
  **Discussions become memories; events never discussed do not.** Importance
  to her and to the owner is tagged, and filters with decay prune below a
  threshold. The world holds her simulated day, local events, and events of
  interest from her work and hobbies — and her social network, constructed and
  updated as part of it.
- **O9 — Her network.** Her own friends, colleagues and family, apart from the
  image library's cast; a UI to view and edit it; edges that update and nodes
  added as new people are mentioned; each edge's context and history, when it
  started or ended; a link from her network to the library.
- **O10 — The owner's network, as she knows it.** She tracks the owner's
  relationships from conversation and updates them; a name she does not know
  is a surprise, and she can ask about it to fill the gap.
- **O11 — No approval queue for news.** The owner does not want to approve
  everything that follows her reading an article. Typed memory of article
  chats by default, and a per-persona switch for full memory (PL23, PL27).

---

## 2. What the evidence says

Researched 2026-10-30/31 by three passes over the web. Figures read from a
primary text are cited to it; a figure taken from an abstract or a secondary
source says so.

### 2.1 What friends talk about

- **Most conversation is about other people.** About two-thirds of casual
  conversation is social topics (Dunbar); a 2025 replication finds ~85%
  ([Cambridge, 2025](https://www.cambridge.org/core/services/aop-cambridge-core/content/view/61BF5C91583F04053BE9471ED96FAE1D/S1866980824000541a.pdf/how-much-conversation-content-is-actually-social-human-conversational-behaviour-revisited.pdf)).
  A persona that talks only about itself and the owner is missing most of what
  friends say. → her network (§6) is where most events come from.
- **Emotional events are retold, then fade.** People share an emotional
  episode in 80–95% of cases, usually several times; more intense episodes are
  shared more often and for longer, then sharing dies out (Rimé,
  [Emotion Review 2009](https://journals.sagepub.com/doi/10.1177/1754073908097189)).
  → retelling is a mechanic with decay (§4.5, §9).
- **Good news is shared most days.** Students told someone their best event of
  the day on 70.8% of days (80.2% in a later study); 57% of positive events
  were social; the worst event was schoolwork (39%) or social (32%). An
  enthusiastic response is what produces the benefit (Gable, Reis, Impett &
  Asher, [JPSP 2004](https://www.sas.rochester.edu/psy/people/faculty/reis_harry/assets/pdf/GableReisImpettAsher_2004.pdf)).
- **Stress is common, majors are rare.** At least one daily stressor on ~50%
  of days at 25, ~30% at 70 (National Study of Daily Experiences,
  [Penn State](https://www.psu.edu/news/health-and-human-development/story/study-finds-experiences-daily-stress-decrease-people-age));
  daily hassles predict symptoms better than major life events (Kanner et al.
  1981, [abstract](https://pubmed.ncbi.nlm.nih.gov/7288876/)). Holmes–Rahe's
  life-change units are a severity ladder for rare events, not a frequency
  prior ([summary](https://en.wikipedia.org/wiki/Holmes_and_Rahe_Stress_Scale)).
- **Mood depends on the activity and the company.** The Day Reconstruction
  Method found commuting, work and childcare least pleasant, and sleep quality
  shifting a whole day's enjoyment (Kahneman et al. 2004,
  [summary](https://news.umich.edu/new-method-measures-emotional-quality-of-daily-experience)).

### 2.2 Time use as a prior

- **ATUS is public domain and episode-level.** The Activity file has one
  record per activity — six-digit code, start, stop, location; the Who file
  says who was present ([BLS data files](https://www.bls.gov/tus/data/datafiles-0325.htm),
  [lexicons](https://www.bls.gov/tus/lexicons.htm),
  [copyright](https://www.bls.gov/opub/copyright-information.htm)). Student
  status is on the respondent file.
- **One day per person.** Each respondent reports a single 4am-to-4am day
  ([BLS](https://www.bls.gov/news.release/atus.tn.htm)), so ATUS gives
  realistic sequences *within* a day and nothing about how one person's days
  follow each other. Weekly structure has to be authored. → the routine (§4.1).
- **The simplest generator is semi-Markov.** Per-hour transition matrices
  plus an empirical duration distribution per activity, conditioned on day
  type and demographic cell; first-order chains get weekday/weekend right and
  miss durations. Dube et al. built exactly this on ATUS at ten-minute
  resolution ([arXiv 2509.18414](https://arxiv.org/abs/2509.18414), abstract;
  no code released). Travel-demand models (ActivitySim, CEMDAP) are far
  heavier than needed.

### 2.3 Simulated lives in agents

- **Generative Agents** planned every agent's day with model calls, retrieved
  memories by recency + importance + relevance (importance a model-rated
  1–10), and reflected when summed importance passed a threshold
  ([Park et al. 2023](https://arxiv.org/html/2304.03442v2)). Failures included
  retrieval misses, embellishment, and 1.3% hallucinated acquaintances; a
  follow-up called its linear cost growth "unacceptable" for long runs
  ([arXiv 2402.02053](https://ar5iv.labs.arxiv.org/html/2402.02053)).
  → a model only at the last step (§4.4).
- **LoCoMo** grounded very long dialogues in persona plus a *temporal event
  graph*, ~600 turns across up to 32 sessions
  ([ACL 2024](https://aclanthology.org/2024.acl-long.747)) — an event backbone
  is what keeps sessions apart.
- **ComPeer** (one week, N=24) gave a companion its own schedule. Users liked
  the sense of a shared life and the topics it raised; they disliked a 7am
  routine message "almost every day", repeated invitations to an activity they
  disliked, an always-studying agent's implicit pressure, and an event told
  twice because duplicates were never removed
  ([arXiv 2407.18064](https://arxiv.org/abs/2407.18064), full text).
  → the told-ledger (§4.5), proactive gates (§12).
- **No controlled study shows that an off-screen life reduces repetition or
  sustains engagement.** The evidence is qualitative and one week to one month
  long. → measure (§14).

### 2.4 Pacing emergent stories

- **Simulate cheaply, tell selectively.** Story sifting runs a simulation that
  produces "a massive chronicle of mostly-uninteresting simulated events" and
  searches it for what is worth recounting (Ryan,
  [thesis](https://escholarship.org/uc/item/1340j5h2);
  [Kreminski et al.](https://mkremins.github.io/publications/AuthoringSifters_TAP.pdf)).
- **Storylets** — small units unlocked by preconditions on state, chosen by
  salience — are the authoring form for a system that picks for itself
  ([Short, 2016](https://emshort.blog/2016/04/12/beyond-branching-quality-based-and-salience-based-narrative-structures/)).
- **Storytellers keep a rhythm.** RimWorld's Cassandra runs ~4.6 days on, ~6
  off, about one major threat every 7–10 days, never before day 11; Randy
  forces a major after a 13-day drought
  ([wiki](https://rimworldwiki.com/wiki/Cassandra),
  [Randy](https://rimworldwiki.com/wiki/Randy_Random)). Valve's director cycles
  build-up, peak and relax (secondary:
  [Game Developer](https://www.gamedeveloper.com/design/the-discomfort-zone-the-hidden-potential-of-valve-s-ai-director)).
- **Traits must be mechanics.** Dwarf Fortress's personality text "was a
  failure" as a story catalyst until it moved behaviour
  ([PC Gamer](https://www.pcgamer.com/games/sim/dwarf-fortress-dwarves-are-more-human-than-human-creator-says-theyre-allowed-to-embody-bigger-emotions-theyre-allowed-to-make-bigger-mistakes-theyre-allowed-to-do-anything));
  The Sims 2 made significant memories more likely conversation topics
  ([StrategyWiki](https://strategywiki.org/wiki/The_Sims_2/Wants_and_Fears)).

### 2.5 What platforms do

- **Off-screen life:** Kindroid and Nomi send proactive selfies and messages
  on inactivity timers; Replika keeps a diary; Character.AI launched a social
  feed in 2025-08 ([TechCrunch](https://techcrunch.com/2025/08/04/character-ai-is-debuting-a-social-feed-within-the-app)).
  Users complain that proactive photos interrupt scenes (Reddit snapshots,
  anecdotal).
- **Current events:** no platform keeps a curated world state per character.
  Kindroid browses per message when the user switches it on, and fetched
  content "can be referenced later"
  ([Kindroid docs](https://kindroid.ai/docs/article/chat-features-and-tools/));
  Nomi reads a pasted link; SillyTavern has RSS and web search extensions with
  a 1500-character default budget and nothing feeding lorebooks
  ([docs](https://docs.sillytavern.app/extensions/websearch/)). None addresses
  injection.
- **Briefing products converge on one shape:** a nightly job, a hard cap,
  items that expire, cheap feedback (ChatGPT Pulse: 5–10 cards a day;
  Gemini scheduled actions: 10 active;
  [TechCrunch](https://techcrunch.com/2025/09/25/openai-launches-chatgpt-pulse-to-proactively-write-you-morning-briefs),
  [Google](https://support.google.com/gemini/answer/16316416?hl=en-GB)).
- **Manipulation is measurable.** An audit of six companion apps found ~37% of
  farewell replies manipulative, raising engagement up to 14× through
  curiosity and anger rather than enjoyment
  ([HBS](https://www.library.hbs.edu/working-knowledge/how-ai-chatbots-try-to-keep-you-from-walking-away));
  heavier companion use tracked more loneliness and less socialising in a
  four-week RCT (N=981, secondary:
  [Fortune](https://fortune.com/2025/03/24/chatgpt-making-frequent-users-more-lonely-study-openai-mit-media-lab)).

### 2.6 News in conversation: the Alexa Prize

The only large-scale evidence from real users.

- **Generic news rated lowest; the user's own thing rated well.** Gunrock's
  News module averaged 3.12 of its 12 modules' ratings (Movies 3.84–3.90), and
  controversial topics drove low ratings
  ([Gunrock 2018](https://d7qzviu3xw2xc.cloudfront.net/alexa/alexaprize/assets/pdf/2018/Gunrock.pdf)).
- **Offer news only on a match.** Chirpy Cardinal offered a headline only
  when an entity already in the conversation matched, paraphrased it in the
  past tense, and talked about the user's favourite team's latest game; its
  opinions came from 1,012 hand-whitelisted reasons on 109 uncontroversial
  topics; neural QA over the article was "a poor and unreliable user
  experience" ([arXiv 2207.12021](https://arxiv.org/pdf/2207.12021),
  [2008.12348](https://arxiv.org/html/2008.12348v2)).
- **Filter at ingest.** Sounding Board filtered "inappropriate & depressing"
  content ([paper](https://ar5iv.arxiv.org/html/1804.10202)); Alana re-ranked
  for recency and dropped repetitive news
  ([paper](https://ar5iv.arxiv.org/html/1712.07558)).
- **Retrieve narrowly.** Naive retrieval was the *worst* condition in
  TimeChara's point-in-time character benchmark
  ([ACL Findings 2024](https://aclanthology.org/2024.findings-acl.197.pdf));
  web retrieval degraded persona fidelity in Amadeus
  ([arXiv 2508.02016](https://arxiv.org/html/2508.02016v4)). A model's
  effective knowledge cutoff varies by topic
  ([Dated Data, COLM 2024](https://arxiv.org/abs/2403.12958)), so its own
  "current events" are never safe to present as current.
- **Personas and sycophancy.** Persona agreeableness correlated with
  sycophancy in 9 of 13 models, though an explicit persona reduced it against
  the generic baseline in most
  ([ACL 2026](https://aclanthology.org/2026.acl-long.1421/), abstract).

### 2.7 Untrusted content and long-lived memory

- **Capacity is a recognised endorsement argument, and unmeasured.** FIDES
  multiplies trust labels by a type lattice `bool ⊑ enum ⊑ string` and calls
  low-capacity outputs "escape hatches" for endorsement — then evaluated with
  the strict policy, not the permissive one
  ([Costa et al. 2025](https://arxiv.org/pdf/2505.23643v2), §5.2, §6). The
  design-patterns paper uses the same argument for map-reduce
  ([Beurer-Kellner et al.](https://arxiv.org/html/2506.08837v3)). No benchmark
  isolates closed-vocabulary extraction. The repo already draws this line.
  Mail triage's record crosses to a privileged reader only through
  `mail_triage::Record::for_privileged_run`. That carries closed vocabularies
  only: `bucket` (3 values), `urgency` (4), `proposed` (7), any subset of
  the nine `TAGS`, and a `request_type`. Together that is about 19 bits a
  thread, before a grounded deadline and the sender's address, and it runs
  over every thread that arrives. The verdict's prose stays behind, and a test
  serialises the record and asserts the prose is absent. The diagnostician's
  episode digests carry tool names, and *those* arm untrusted, because "a
  sentence can be shaped like an identifier" (`TRIFECTA.md`, the switches
  table).
- **The theory adds three conditions.** The schema and the decision to endorse
  must be fixed by code, never chosen by a model that read the content (robust
  declassification, Zdancewic & Myers 2001); the extractor must see only the
  content, never private data, or the endorsed value becomes attacker × secret
  (transparent endorsement, [Cecchetti et al. 2017](https://arxiv.org/pdf/1708.08596));
  and bits add up — n values carry n·log₂k bits, which is how one exfiltration
  spelled data through 26 fixed URLs
  ([The Register, 2026-01](https://www.theregister.com/2026/01/08/openai_chatgpt_prompt_injection/)).
- **Capacity does not bound truth.** An attacker who cannot inject can still
  choose a wrong value. Five planted texts per question reached ~90% attack
  success on retrieval ([PoisonedRAG](https://www.usenix.org/conference/usenixsecurity25/presentation/zou),
  abstract); false claims in BlenderBot's long-term memory made it 328% more
  likely to repeat them ([arXiv 2304.05371](https://arxiv.org/abs/2304.05371)).
- **Summarising does not launder.** CaMeL names manipulated summaries a
  non-goal and assumes memory is uncompromised
  ([Debenedetti et al. 2025](https://arxiv.org/pdf/2503.18813), §3); a writer
  in a two-agent memory relay propagated attacks at 53% for a small model
  (abstract, [arXiv 2603.28013](https://arxiv.org/abs/2603.28013)). A local
  writer should be assumed weak.
- **Detectors do not hold.** Adaptive attacks broke all twelve published
  defences, mostly above 90% ([arXiv 2510.09023](https://arxiv.org/abs/2510.09023),
  abstract). **No vendor gates memory writes**; the shipped fixes closed
  exfiltration and left the write
  ([Gemini](https://embracethered.com/blog/posts/2025/gemini-memory-persistence-prompt-injection),
  [Anthropic memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)).

### 2.8 What none of this proves

That a world makes a persona better company. §14 measures it before the
expensive parts are built.

---

## 3. The shape: a world beside the memory

```
                 ┌──────────── world.db (per persona) ──────────────┐
 routine       ─▶│ day log      sampled days, hours by activity     │ pruned in days
 her network   ─▶│ people, ties, edge events                        │ minor people decay out
 storylets     ─▶│ events       simulated, local, interest          │ pruned by score
 feeds A / B   ─▶│ arcs         ongoing situations and progress     │ closed arcs summarised
                 │ state        job, relationship, home, health     │ persists until superseded
                 └───────────────┬──────────────────────────────────┘
                                 │ the today-block (fixed budget) → a chat
                                 │ told in a chat?  → a snapshot + pointer
                 ┌───────────────▼─── memory.db (exists, §9) ────────┐
                 │ episodes: "we talked about it" + snapshot, pointer│ decays from recall
                 │ persona facts · owner facts · the owner's people  │ never silently deleted
                 └───────────────────────────────────────────────────┘
```

Two stores with different jobs:

- **The world is everything that happens to her.** Machine-written, decaying,
  never the ground truth of anything the owner saw.
- **Memory is only what happened in a chat** — unchanged from
  `PERSONA-DESIGN.md` §9, which keeps its provenance story, `forget` cascade
  and curation exactly as they are.

**An undiscussed event never becomes a memory.** It stays in the world until
its score decays, then it is gone. Nobody saw it, so nothing is lost and
nothing she says later can contradict it. **A discussed event is copied into
memory** — that it was talked about, how the owner reacted, a typed snapshot
of the event and a pointer to it (§5) — and the world copy may then be pruned,
because memory carries what it was.
Her past, as the owner knows it, is consistent by construction: she can only
retell what the owner heard, or what is still fresh in her world.

**Groups share a world the way `shared.db` shares facts** (§4.5 of the
persona design): a group's `world.db` holds places, local events and shared
people; each persona's own world holds her life on top. What a persona *knows*
stays hers.

---

## 4. The life simulator

Five layers. Only the fourth uses a model.

| Layer | Does | Written by |
|---|---|---|
| 1. Routine | Weekly anchors: classes, shifts, practice, lectures, a standing dinner | The owner (a model may draft it, §4.1) |
| 2. Day sampler | Fills the hours around the anchors; who was present | Code, seeded |
| 3. Storyteller | Draws events from storylets; the director paces them | Code, over an authored deck |
| 4. Narrator | Writes up the few events the code chose | The local model, nightly |
| 5. Teller | What reaches a chat, and what the owner has already heard | Code |

### 4.1 The routine

Owner-approved structured text beside `persona.toml`: anchors by weekday and
time, with a probability where real life is soft ("gym, Tue/Thu 7am, 0.7").
A model may **draft** a routine from `identity.md`, staged as a candidate and
approved on the Personas page as `persona_propose` candidates are
(`persona::approve_as_shown`); nothing reaches a sampler until the owner
approves (§16 PL1). A term calendar, holidays and seasons ride in the same
file or a group's.

### 4.2 The day sampler

A semi-Markov chain over ATUS tier-1/2 activities (§2.2), estimated once from
the public files for a handful of demographic cells and shipped as a table:
per-hour transition probabilities, an empirical duration distribution per
activity, and a who-was-present distribution from the Who file. Each day:

1. Place the routine's anchors (each drawn against its probability).
2. Fill the gaps from the chain, conditioned on day type and cell.
3. Draw who was present for each episode from her network (§6), weighted by
   closeness and the Who file's distribution for that activity.

Seeded per persona per day (a seeded draw in `sample.rs`'s manner, without
widening that module's holdout role), so a day is reproducible and a
replay can recompute it. **Traits move the numbers**, never just the prose
(§2.4): an adventurous persona's novelty weight, an athlete's workout slots,
a night owl's shifted sleep.

### 4.3 The storyteller and the director

**Storylets** are small authored events with preconditions over state and
effects on it — "at practice ∧ fatigue high → a minor injury (−mobility,
arc: recovery)"; "dinner with a close friend → good news shared (+closeness)".
A starter deck ships per broad life shape (student, parent, clinician,
academic, office worker), owner-editable like relationship templates; the
owner's deck is the owner's.

**The director** paces them (§2.4):

- A **floor** of minor events: a hassle or an uplift most days, at roughly
  the diary-study rates (§2.1).
- **Majors** on a cycle with a minimum gap, a forced quiet spell after a
  peak, and a forced event after a long drought — one budget of drama shared
  by every source, so a big game, a fight with a friend and a hard shift
  compete rather than pile up.
- A **severity cap**: Holmes–Rahe-scale events (a death, a divorce, a serious
  diagnosis) only on a per-persona opt-in (§16 PL3).
- **Most events involve other people** (§2.1): the director weights storylets
  with a cast member above solo ones.

### 4.4 The narrator

A sifter picks the day's one to three notable events — by importance to her
(§9), novelty against recent events, and the director's budget — and the
local model writes each up in a few sentences: what happened, who was there,
how it felt. Everything else stays a typed record nobody narrates. That is the
whole of the model's role: code decides what happened and when; the model
writes how it felt.

It runs nightly beside `persona memory write`. The narrator is not per chat,
but `writer::pick_model` already has the shape it needs: called with the
persona's pinned model (`PERSONA-DESIGN.md` §12.6) against the router's
resident one, it answers `Use` when they match or the router is empty and
`Wait` when another model is resident. The narrator calls it as-is. Otherwise it waits for a later night, never
swapping a model (the 2026-09-27 ruling). Its inputs are owner files, code, typed records from shelves A and B
(§8), and **only those world rows whose origin is clean**. Chat-sourced rows
from a clean stretch qualify. A row written under taint does not, *even after
the owner approves it*. Approval makes a record recallable, never clean:
`PERSONA-DESIGN.md` §9.6 has an approved record re-arm the taint it was
written under. So the narrator never reads such a row, and its output is
`ModelClean`. **The simulated life has no taint problem.** Only shelf C does,
and shelf C never reaches the narrator.

The narrator's prose is long-lived: it is snapshotted into memory (§5) and
rides in later prompts. So the claim that it is clean rests on its inputs, and
specifically on shelf B being typed. §2.7's "summarising does not launder"
concerns prose inputs, and the narrator has none. What reaches it from outside
is a handful of enum values under §8.2's ceiling, which can tilt what it
writes but cannot spell an instruction into it. If shelf B ever gained a
free-text field, the narrator's output would stop being `ModelClean`. That is
one more reason §14's gate part 1 exists.

### 4.5 The teller: the today-block and the told-ledger

- **The today-block** rides in the message stream, as the recall block does
  (`PERSONA-DESIGN.md` §8.2) — never the system prompt, so it never costs the
  cached prefix. Under a fixed budget it holds: what she is doing now (the
  clock is already per turn), what is on her mind (top events and arcs by
  importance and freshness), her mood, and the people she saw today.
- **The today-block arms taint at the fold, under its own stems.**
  `PERSONA-DESIGN.md` §9.7's rule governs any fold: the text arrives in no
  tool result, so the harness arms taint where it folds, and
  `Taint::arm_for_content` must be able to re-derive it from the transcript at
  every run start. `arm_for_content` already recognises six folds. Two of
  them are clean/untrusted stem pairs (`persona::recall::stem_of` and
  `scene::stem_of`), because one stem arming both "would have made every chat
  that remembered anything untrusted." The today-block is a seventh fold and
  takes the recall pair's shape:
  - **the clean stem arms `private`.** Most of the block is code, owner
    files, clean narration and shelves A and B. But stated plans and people she
    mentioned in a chat ride in it too (§13): a plan the two of them made is
    memory of a conversation with the owner, the case §9.7 arms `private` for.
    Arming is close to free here, since D11 already lifts the leak guard in
    persona chats and a persona's `web_search` is blind at every depth
    (`PERSONA-DESIGN.md` §3.3). The alternative, keeping chat-sourced rows out
    of the block and sending them through the recall block, splits one day
    across two folds for no gain.
  - **the untrusted stem arms `private` and `untrusted`** the moment any
    folded row's origin is not clean.

  The untrusted variant exists from the first build, so a dirty result at the
  shelf B gate (§14), or a candidate row approved by mistake, needs no new fold
  (§16 PL20).

  **There is a ratchet, stated rather than hidden.** Approval makes a row
  recallable, never clean (§4.4). Once the owner approves one untrusted-origin
  world row, every later chat that folds it opens under the untrusted stem,
  and §9.6 makes that chat's memories candidates. The recall block already
  behaves this way and it is accepted there, but it lands on shelf C's main
  path, beside §8.4's cost.

  **The today-block rides as its own run note, never concatenated with the
  recall block.** The stem pairs this fold is shaped on (`recall::stem_of`,
  `scene::stem_of`) match a prefix after trimming. (The appraisal and digest
  stems match with `contains`, because they sit mid-brief.) If two folds
  shared one text block, only the first stem would sit
  at the start, and the second would never arm on a resume, failing in the
  open direction. As its own note it works: `Taint::arm_for_notes` gives each
  note its own block before matching, and `Record::Notes` re-arms each one on
  load, as it already does for the recall block. A new stem is taught to the
  two predicates the recall and scene stems are in: `arm_for_content` (which
  the `Record::Notes` re-arm reaches through `arm_for_notes`) and
  `message::is_recorded_note`. The second keeps a per-run note that somehow
  got stored off the wire, which the scene fold already relies on.
- **The told-ledger** is the anti-repetition mechanism. The nightly writer
  already reads each chat; it marks which world events and people she
  mentioned. An untold event is fresh material. A told one fades as Rimé
  describes and is never re-introduced, only followed up ("the ankle's better,
  by the way"). ComPeer's repeats came from exactly this check missing (§2.3).
  - **The `told` flag carries the taint of the stretch it was read from.** It
    is a write to an *existing* world row, so the row's own origin says
    nothing about it. After a `news_read`, an ungated flag would be the one
    world write that decides what she has left to talk about, and both
    failure directions look like the feature not working: marking
    unmentioned events told takes away her material, and leaving mentioned
    ones untold makes her repeat herself.
  - So the rule is asymmetric, in the safe direction. A `told` set from a
    tainted stretch **is applied**, because suppressing is the safe
    direction. A tainted stretch **never clears** a flag. Each flag records
    its source chat and taint, so the week view can show which events went
    quiet from a tainted chat (§13).
  - **The set is bounded by what the chat was shown.** A tainted stretch may
    set `told` only on events that rode in *that chat's* today-block or recall
    block, never on arbitrary world rows. So an injection that makes her
    "mention" things can silence at most what was in front of her, a handful
    of events per chat. This channel is outside the reach of §14's steering
    arm, which drives a feed, not a chat, and §14 says so.
- **Stated plans flow back.** When she says in a chat that something will
  happen ("there's a game on Saturday"), the writer records it as a **stated
  plan**, and Saturday's simulated day contains it. Without this, chat and
  world disagree within a week.
  - **These are not commitments in `guilt.rs`'s sense, deliberately.** That
    module's rule is "an expectation is a recorded commitment, never a claimed
    one": a model *saying* something creates no obligation, and its stores
    are all owner-visible queues. A stated plan is the opposite case, a
    model's claim written into her world. It shapes a simulation and owes
    nobody anything, so it is named differently to keep the two apart.
  - **After a taint rise, a stated plan is a candidate**, as §13 and §6.2
    already say for every mention of hers in a tainted stretch. It does not
    enter the world until the owner approves it. Candidates show on the week
    view as *waiting plans*, so a news-following chat whose later plans
    never happen is visible rather than silent.
  - Whether a stated plan from a tainted stretch could be admitted instead,
    as a typed record (a date, an activity from a closed set, people ids) that
    is checked against the turn it was said in, is an open decision (§16
    PL21). The argument for: it is low-capacity and verifiable. The argument
    against: it is the persona's own utterance after an injection.

---

## 5. The world store

`<persona>/world.db`, SQLite, machine-written, beside `memory.db` — the
conditions `PERSONA-DESIGN.md` §4.3 names for leaving files (it grows, it is
read every chat, several processes write it).

| Table | Holds | Lifetime |
|---|---|---|
| `days` | Sampled episodes: activity, start, end, place, who was present | Hard-pruned after ~two weeks; a per-week summary row survives |
| `events` | `{when, kind, storylet, people, arc, importance_her, origin, source, told}` | Pruned by score (§9) |
| `arcs` | Ongoing situations with a stage and a next-step distribution | Closed arcs collapse to one summary event |
| `state` | Slow facts about her life: job, relationship status, home, health | **Persists**, superseded by a later row; never decays |
| `people`, `edge_events` | Her network (§6) | Minor people decay out (§6.3) |
| `items` | Interest shelves A and B, and pointers to C (§8) | Expire in days |

**State vs events** is what makes aggressive pruning safe. "She changed jobs"
is state; the leaving-drinks evening is an event. Events decay, state is
updated — so decay removes texture, never a fact about her life. This mirrors
the tiers ruled in D26 (momentary, short-term state, ongoing situation, lasting
fact): an event is momentary, a mood is short-term state, an arc is an ongoing
situation, a state row is lasting.

**Every row carries an origin and a source** (owner edit, storylet, chat turn,
feed item), which is what `forget` on a chat works from.

**D27 is a rebuild that memory doesn't yet have, and the world can do it
cheaply.** D27 ruled "rebuild the fact from the chats that remain"; until that
re-derivation exists, a memory fact goes with any of its chats. The world must
not inherit that blunt fallback, because the network is a fold over events
from several sources (§6.1), and dropping a whole edge would take
owner-authored events with it. So the world does the rebuild D27 describes:

- `forget` deletes the rows and edge events **sourced to that chat**, then
  re-folds what remains. Owner events are never removed by forgetting a chat.
- A person who exists *only* through that chat's events goes with it.
- A stated plan from that chat is deleted.

The event log makes this nearly free, as it makes §15's time slider free, so
the network may become the first store where D27's rebuild is cheap (§16 PL22).
The re-fold is pure code, with no model run anywhere in the erase path. That
is the condition §9.13 of the persona design gives for why the blunt fallback
stands today.

**D27 has a second half, and the re-fold honours it.** D27 also says "back to
a proposal if too few [chats] do [remain]".
- **A single event needs no fallback.** It belongs to exactly one source, so
  dropping it is the whole answer.
- **Derived values do.** Closeness, warm or tense, active or ended are folds
  over several events. When a forget drops a derived value's support below a
  floor (for a sim or chat edge, fewer than two remaining events), the value
  **reverts to a candidate** rather than standing at its old strength.

**Pointers into the world can dangle, and are reported by name.** World rows
are pruned while memory keeps pointers to them. A dangling pointer is shown as
a broken link reported by name, the house rule of §6.5, never an error or a
silent gap.

**A told event is snapshotted into memory, not merely pointed at.** When the
writer copies a discussed event into an episode, it stores a typed snapshot
with the pointer: kind, date, people ids, and the narration, which is
`ModelClean`. After that the world row may be pruned and the episode still
says what it was. **The snapshot's narration names people by id, never by
name.** The narrator writes placeholders, and names are rendered at recall
from the nodes that remain. A person forgotten with chat B (§6.2's
chat-sourced origin) then renders as "someone", rather than surviving as a
name in chat A's snapshot. That is `PERSONA-DESIGN.md` §9.13's rule: "that
sentence may be the last trace of what was erased". It also lets the
dangling-pointer rule above cover names, since a broken link reported by
name works on an id and not on a sentence. Shelf C is the exception: an episode keeps only its pointer
(§8.4), so nothing an article's author wrote is copied anywhere.

---

## 6. Her network

### 6.1 A graph built from events

The network is an **append-only log of edge events, folded into current
state** — never an edge table rewritten in place. That gives history, start
and end dates and "how did this change" for free, and it is how the session
store and `guilt.rs` already work.

- **People:** name, aliases, role in her life, a line of description, an
  optional library character (§6.5), origin (owner, chat, simulation), and
  `known_to_owner` — whether she has ever mentioned them.
- **Edges:** ego edges (her ↔ someone) and **ties** between two people in her
  world, where most gossip lives. A closed type with a free label beside it —
  kin (spouse, child, parent, sibling, …), romantic, ex, friend, colleague,
  mentor, neighbour, acquaintance, rival. Closeness may be held per direction.
- **Edge events:** `{edge, kind, valid_at, recorded_at, precision, context,
  source, origin}`, where kind is met, formed, strengthened, conflict,
  reconciled, paused, ended or renewed.
  - **Two dates.** "We've been friends since school," said in a chat, has a
    `valid_at` years back and a `recorded_at` of today. Precision is day,
    month, season, year or unknown, so a vague date is never forced into a
    false one — `capture.rs`'s rule: *when* is detected and reported, never
    resolved.
  - **Context** is a line: how they met, what the conflict was about.
- **Current state is the fold:** active or ended, closeness as a Dunbar layer
  (5 / 15 / 50 / 150), warm or tense, last contact, started and ended.

### 6.2 Where people come from

| Source | Written by | Lands as |
|---|---|---|
| The owner | The People tab | Origin `owner`; the simulation may update their **state**, never who they are |
| She mentions someone in a chat | The nightly memory writer | Active if the chat's stretch was clean; a **candidate** if tainted (§9.6's split, unchanged) |
| The simulation needs someone | The storyteller: a name from a list, a line from the model | Origin `sim`, outer layer |
| The owner mentions *their own* people | Not this graph | §7 |

**Promotion is the owner's.** A chat or sim person who becomes central is
proposed for owner status through the candidate flow. **Mentions make the
world honest:** until `known_to_owner` flips she *introduces* someone ("a
friend from my study group, …"); afterwards she can name them in passing.

### 6.3 Decay

Owner people never decay. Chat people decay slowly and only from recall
ranking, as memory does (§9). Sim people never mentioned decay out of the
outer layers when untouched and are pruned below a floor, like any event.

### 6.4 Identity: when two mentions are one person

An exact alias match is not an identity match — the owner's graph learned
that importing a CV. So:

- An alias match **within this persona's network** with a compatible role
  merges.
- Anything ambiguous (two people of one name, a nickname, a role conflict) is
  a **proposed merge** for the owner; a model never decides it.
- Merge and split are on the People tab and reversible, because the event log
  keeps who said what.
- **A contradiction with an owner person does not apply.** If she claims two
  children and the owner wrote one, the writer flags it and changes nothing.
  Rendering the core network into the chat-start block makes this rare.

### 6.5 The library link

- A person may name a **library character** — the name-as-link pattern of
  `PERSONA-DESIGN.md` §4.1, a broken link reported by name. This is what
  pictures use: a photo with her partner casts the partner's portrait. One home
  per attribute holds: looks and voice in the library, the relationship in the
  network, personality in the persona.
- A frequent person without one can have a character **proposed**, approved in
  the same tap `persona_propose` already uses for a waiting character.
- **The library shows where a character appears** across personas' networks —
  the owner's view only. Personas still do not know each other (§8.1 of the
  persona design): two nodes linking one character share a face, never a
  memory.
- **People shared across personas live in a group's world** (§3), so two
  personas in one town agree on who exists while each keeps what she knows.
- **Locks carry over.** A locked character is never named on a visible
  persona's network; the link is hidden, never cut (§8.3 of the persona
  design).

### 6.6 Storage, and a deviation

The network has **mixed authorship**, and `PERSONA-DESIGN.md` §4.3 puts what
the owner writes in TOML and Markdown. The recommendation is the network in
`world.db` with a per-event origin and owner precedence — the generated layer
never overwrites an owner event — plus a TOML/Markdown export, rather than a
`people.toml` the UI rewrites and the simulation overlays (§16 PL5). People
already written into `identity.md` are proposed once as owner nodes; after
approval the network is their home and `identity.md` keeps personality, so the
fact lives in one place (§16 PL6).

---

## 7. The owner's network, as she knows it

### 7.1 What it is

A structured layer over the facts about the owner that already exist
(`PERSONA-DESIGN.md` §9.5: stated and inferred, private to the persona unless
the owner shares them, D17/D18) — the people-shaped part, as a second graph
centred on the owner. Same event model and the same two dates as §6.1.

**It lives in `memory.db`, not `world.db`.** It is memory of the owner, so it
follows memory's rules rather than the world's: it is soft-pruned out of recall
and never hard-deleted (PL7). A graph of the owner's real relatives silently
hard-pruned by a world threshold would be the wrong half of that split.

**`forget` here is a decision of its own** (§16 PL24). D27's rule *today* is
the blunt fallback: a fact goes with any of its chats. Re-folding this graph
from the events that remain, as §5 does for the world, is the rebuild D27
describes, arriving in memory. For a graph of real relatives the two rules
give materially different results. Forgetting the chat where the owner first
named a sibling would delete the sibling under the fallback, and keep them
under the re-fold, with only that chat's events removed.

**The proposal half matters most here.** §7.4's `Inferred` edges are accepted
automatically under D18. An inferred tie that stood on three mentions and now
stands on one is exactly D27's "too few remain", so it reverts to a
candidate. The owner is asked before a weakened inference about a real
relative stands again.

| | Her network (§6) | Her model of the owner's |
|---|---|---|
| Who | Fiction | Real people |
| Learned from | Owner edits, her chats, the simulation | **Only what the owner says in chat** |
| Kind | — | `Stated` and `Inferred`, always shown apart |
| Shared | Through a group's world | Only when the owner shares it (D17) |

**The two graphs never share a node.** Fiction can never leak into what she
believes about the owner's life, or the reverse.

**She never reads the owner's stores.** The owner's knowledge graph already
holds the owner's network, and §3 of the persona design gives a persona no tool that
reads it. She learns the owner's people as a friend does, one conversation at a time.
The other direction is D10: never by default; per persona, with permission,
staged to the graph's review queue.

### 7.2 Recognition and gaps

1. **She knows who she knows.** The chat-start block carries a compact
   roster — name, relation, a line. Each turn, the harness matches names in
   the *owner's* message against it and folds a card for each match (relation,
   the last thing the owner said about them, open threads): the recall rule, "the
   owner's words key the search", applied to people.

   **Both folds are memory of the owner, so both arm `private`.** They carry
   the owner's real relatives and friends, and §9.7's reasoning for the recall
   block applies word for word. They ride **inside the recall block, under
   `persona::recall::stem_of`'s existing pair**: the clean stem arms
   `private`, and the untrusted stem arms both when any folded fact's origin
   is not clean. That is one fold `arm_for_content` already re-derives, rather
   than a new one. It matters beyond bookkeeping: the writer's §9.6 split
   classifies by what was armed. (Two things are not reasons. A persona chat
   forces `block_sends_after_private` off (`persona::agent::security`, D11),
   so arming has no send-blocking effect. And the blind path is not one
   either. A persona's
   `web_search` is built with `blind_only` (`WebSearch::for_persona`) and
   takes the blind chain whatever is armed, §7.5.) A roster folded
   silently would leave the owner's social graph in context with nothing
   recording that it is there.
2. **An unknown name is a gap.** A capitalised name matching no one gets a
   harness note, and she may ask. The writer also records it nightly in a
   **gap ledger**, so it never depends on her noticing live.
3. **Gaps are ranked and expire.** Priority is mentions × emotional charge ×
   recency. A name said once in passing decays unasked; a name said three
   times with feeling becomes "you've mentioned them twice now — a friend from
   work?" Gaps can be partial: a known relation with an unknown situation.

### 7.3 Asking without interrogating

A month-long study found a *probing* proactive character "pushy" and a
horizontal one "a mind of its own" ([arXiv 2601.22452](https://arxiv.org/html/2601.22452v1)).
So: at most one gap question a chat (§16 PL17); none while the owner is upset
or mid-story; asking how someone *is* over what they *are*; and follow-ups
days later ("how did the move go?") — the capitalization finding: friends
remember to ask.

### 7.4 History on the owner's relationships

The same edge events — met, close, conflict, drifted, ended, renewed — each
pointing at the turn it came from. **Stated and Inferred stay visible
everywhere**: "you said you fell out" is a different claim from "you seem to
be drifting." Inferences follow D18 — accepted, shown as hers, one tap to
correct or delete — and the owner's correction becomes a `Stated` event.

### 7.5 Guardrails

These are third parties who never agreed to be in a database.

- **A filter on searching for the owner's people, which is a cost narrowed,
  not a boundary held.** A `web_search` query containing an exactly spelled
  name from this graph is refused (§16 PL14). That catches the accidental
  case and nothing more. The adversary that matters is the persona's own model
  composing the query under injection, and this repo has measured a model
  routing around a string filter by misspelling (`PERSONA-DESIGN.md` §12.7, on
  DRY). A nickname, an initial, a transliteration or a name split across two
  queries all pass.
  - **What actually holds** is structural, and already ruled: a persona's
    `web_search` always takes the blind path at quick depth, so no destination
    is the model's choice. §3.3 of the persona design already states the
    residual plainly: a query can carry what the persona remembers to the
    configured search backends.
  - **The filter's own costs:** the owner's-network graph is loaded at every
    search, and a name that collides with a common word or a public figure
    refuses a legitimate search.
- **No faces by default.** A library link for one of the owner's people exists only if
  the owner makes it (§16 PL16). Even then, **the world never casts it**:
  the owner's people are not in her simulated days, and neither the camera
  roll nor any storylet may draw them. A render of a real person happens only
  when the owner asks for that picture in a chat. That stays consistent with
  `PERSONA-DESIGN.md` §18's consent argument against real people as personas.
- **She supports the owner's relationships and never competes with them.** A romantic
  persona with a map of the owner's relationships is exactly what could
  position itself against them (§2.5). The recommended form is a safety-layer
  sensor (`PERSONA-DESIGN.md` §12), on by default and switchable like the
  rest: disparaging or isolating remarks about people in the owner's network,
  recorded beside the dose meters (§16 PL15).
- **Locked and plaintext.** It lives in her folder under the library lock and
  is plaintext on disk; the page says so (§8.3 of the persona design).

---

## 8. Interests and current events

### 8.1 Depth and freshness

- **Depth** — what she knows about her work and hobbies — is the model's own
  knowledge, steered by an interests section in the persona. Free and clean,
  but stale: a persona without fresh sources is told plainly that it has no
  recent news, because a model's effective cutoff varies by topic (§2.6).
- **Freshness** comes from feeds the owner registers, per persona or per
  group (§16 PL11), **in the persona's or group's folder under
  `~/.mecha/personas/`, never in layered config**. A nightly, unattended fetch
  target is the class CLAUDE.md keeps out of layered config, like a cron slot,
  so a cloned repo can never add one. If a global table is ever wanted, it is
  global-layer-only and stripped from a project layer loudly, as `[[mcp]]
  personas = true` is (`PERSONA-DESIGN.md` §3.3). Feeds are fetched nightly
  **by the harness, never by a model** — the
  HUD's rule for loaders: one read of one owner-registered source. The fetch
  is **capped at the door**, the way `image.rs` caps an image: a feed larger
  than its limit, or with more items than its limit, is refused by the fetcher
  and named on the reading list. It is never truncated into the extractor's
  context.

### 8.2 Three shelves, sorted by capacity

| Shelf | Holds | Produced by | Trust | Drives the world? | Enters memory? |
|---|---|---|---|---|---|
| **A — parsed facts** | Scores, airdates, weather, release dates, new papers in a category | **Code**, from structured sources (sports and TV listings APIs, the weather service, arXiv metadata), parsed **into closed types with refusal** (below) | Clean only as rendered: integers, dates, registry ids, closed taxonomies | Yes | Yes |
| **B — typed gist** | `{kind, tone, entity}` for prose feeds, plus fields code supplies (below) | A quarantined pass that sees **only the article** (`quarantine.rs`), under a schema fixed in code | Clean by capacity, under the stated bit ceiling | Minor events only; serious kinds (deaths, disasters) need shelf A or stay on C | As pointers |
| **C — prose** | Headline, a short neutral summary, a link | The same pass | **Untrusted**; expires in ~five days | Never | Never as text |

**Shelf A is clean only because its parse is closed, and it is built first,
so its rules must hold before any of §14's gate exists:**

- **Every field is parsed into a closed type, or refused.** That means an
  integer, an ISO date, an id in the owner's registry, a value from a
  taxonomy fixed in code (a sport's result codes, arXiv's category list), or
  an id matching a shape fixed in code. An unparseable or unresolvable value
  is **dropped and named on the reading list, never stored as a string**.
  That is "unknown is never clean" on the shelf that claims to need no gate.
- **Shape-matched ids are stored, never rendered.** A new paper's arXiv id
  cannot be in any registry, and the feed chooses it. It is kept as a pointer
  for the reading list. What renders is the closed part: "a new paper in
  [category]". A title is prose, so it is shelf C. Without this rule, shelf A
  would be a wider path than shelf B on a weaker argument, the shape
  `digests_in_brief` arms untrusted for.
- **Shelf A's argument is provider trust, bounded by render ranges, not
  capacity alone.** A closed type is not a small one: a score is an unbounded
  integer and a date is about 15 bits, unbounded. What shelf A really leans on is that its sources are
  structured APIs the owner registered, which serve data, not
  attacker-authored prose. That is a different claim from shelf B's, so it is
  stated as one. What makes it safe to lean on is a **per-field render
  bound**, fixed in code:
  - an integer has a plausible range stated per field: a score is 0–200
    (about 7.65 bits), a temperature is a whole degree Celsius from −40 to
    50 (91 values, about 6.5 bits);
  - a date falls within **±7 days of today** (15 values, about 3.9 bits);
  - an enum comes from its taxonomy, sized by its entries;
  - an id from the owner's registry follows shelf B's rule (at most 31 plus
    none, 5 bits).

  An out-of-range value is dropped and named on the reading list, like any
  unparseable one.

  **Shelf A has stated figures, as shelf B does, so its test has a constant
  to fail against:**
  - **at most 16 bits per item**, summed over the item type's rendered fields
    at their bounds. For example, a game result rendered as two scores and a
    date costs 15.3 + 3.9 ≈ 19.2 bits, which fails. So it renders as a win,
    loss or draw (1.6 bits), a margin band of eight (3 bits), the date (3.9)
    and the opponent's registry id (5): about 13.5 bits, which passes. A
    weather item, a temperature and a date, is about 10.4 bits;
  - **at most three shelf A items a day**;
  - **expiry fixed at five days**.

  That gives **under 48 bits a day and under 240 live at once**. Gate part 2
  computes every shelf A item type's ceiling from its ranges and fails the
  build if a type exceeds 16 bits, the same way shelf B's vocabularies are
  checked, so an integer field cannot slip past the bit test for want of a
  vocabulary (§16 PL9).
- **Gate part 1 covers shelf A from its first build.** The conformance test
  (§14) runs over shelf A's parsers too: a canary in any feed field must
  never reach the world or a prompt.

Shelf B meets §2.7's three conditions in code:

- **Robust declassification.** The schema is a Rust type, and the decision to
  endorse is the type itself; no model chooses either.
- **Transparent endorsement.** The pass is a `quarantine.rs` one-shot with no
  tools, no history and no persona context.
- **A stated capacity.** Only the fields the article could actually influence
  count, so code supplies everything it can:
  - **interest** comes from which feed the item arrived on, the owner's
    mapping, so it carries 0 attacker bits;
  - **salience** is computed by code from kind and entity, never extracted
    (0 bits);
  - **the order** of the items is a lexical sort over the extracted values
    themselves, adding nothing. **The count does add something**: whether 0,
    1, 2 or 3 items survive is up to the publisher, about 2 bits a day. That
    sits outside the per-item ceiling, with the timing bit (§14 part 3);
  - **kind** is one of 8 (3 bits) and **tone** one of 3 (1.6 bits).
  - **entity** is *an index the pass picks into a candidate list that code
    supplies with that feed*: at most 31 public names the owner registered for
    that feed's interest (a team's roster, a field's labs), plus none, for
    5 bits. The pass sees only that feed's list, never the owner's registry as
    a whole, and never anyone from either network.

    **This list is still owner-chosen data in the extractor's context**, so
    "narrower" is not what makes it pass §2.7. Two other facts do:
    - **The list is public vocabulary, not facts about the owner.** It holds
      names anyone following that interest knows: a roster, a field's labs.
    - **The index cannot be read back out.** With no `Chosen` sender
      (`PERSONA-DESIGN.md` §3.3), attacker × list can only steer what she
      raises, never reach the attacker.

    **Its residual:** `web_search` stays on after a `news_read` (D11), and a
    blind query still reaches the `[[search]]` chain. So an entity choice
    worth about 5 attacker bits can reach the search backends, which is the
    cost §3.3 already names.

    Two other shapes fail §2.7 outright:
    - letting the pass resolve names against the full registry puts owner
      data in its context, so the endorsed value becomes attacker × secret;
    - letting it emit a name for code to resolve sends an attacker-authored
      string across the boundary, which is what §14's gate part 1 exists to
      catch (§16 PL9).

  - **Which items fill the slots is code's choice, not the feed's.** Any
    selection based on extracted fields (salience, an entity match) would
    hand the feed the choice of which of its N arrivals survive. That is
    another log₂C(N,3) bits a day, about 15 at N = 50, and §14's bit test
    could not see it, because a selection channel has no vocabulary. So the
    three daily slots are allocated **round-robin across registered feeds**,
    which fixes how many slots each feed gets, before extraction is even
    consulted.
  - **Within a feed, the pick is a seeded draw, not the feed's order.** A
    feed's item order belongs to whoever authors it, so "first in the
    document" would hand an attacker the slot. Code instead draws uniformly
    from the feed's admissible items: those whose published date parses and
    falls within the fetch window. The seed is per persona per day, not
    derivable from the feed.
    - For a wholly attacker-authored feed, selection adds nothing over 9.6
      bits: the attacker writes the surviving item anyway.
    - For one planted item in an honest feed of N, the draw gives it a 1-in-N
      chance at a slot, rather than the certainty first position would. That
      is §2.7's PoisonedRAG case.

    The only attacker-chosen bits are then the surviving items' own fields.

  That puts the ceiling at **about 9.6 bits per item**. With **at most three
  shelf B items a day** per persona, the ceiling is **under 30 bits a day**.
  Shelf B items **expire at five days**, a figure fixed for this shelf, so
  the ceiling is **under 150 bits live at once**.

**Measured against what already ships, that is conservative.** Mail triage's
privileged record carries about 19 bits per thread from closed vocabularies
alone, over every thread in an inbox (§2.7). Shelf B carries half that per
item, over at most three items a day.

What bits can do in a persona chat is narrower still. A persona registry holds
no `Chosen` sender, so the trifecta interlock has nothing to refuse
(`PERSONA-DESIGN.md` §3.3). The bits cannot be read back out; they can only
*steer* what she raises, her mood, or the director's choices. The steering arm
(§14) measures that residual directly.

A test computes the ceiling from the shipped vocabularies, and a vocabulary
change that raises it past the stated figure fails the build (§16 PL9).

**What capacity cannot bound is truth.** An attacker can still choose a wrong
value, which is why serious kinds stay off shelf B.

**Structurally, this is mail triage one directory over.** Shelves B and C come
from one pass, as one record with a clean half and an untrusted half. A single
constructor, the counterpart of `mail_triage::Record::for_privileged_run`, is
the only way the clean half reaches the world or a prompt, with the same test:
serialise what crosses and assert the headline and summary are absent. That
makes it a gate in the type rather than a rendering convention. It is also why
the headline never rides in the today-block.

### 8.3 How she uses them

- **In the today-block** (clean): a game result (A); "good news about one of
  your interests" (B). The storyteller may turn A and B into lived events —
  watching the game with friends — that she raises on her own and memory may
  keep.
- **On demand:** `news_read(item_id)` returns shelf C's prose inside
  `<untrusted-content>`. **Taint rises at that turn, honestly**, and §9.6's
  split keeps everything before it clean. Three details carry the "honestly":
  - **Both halves of the loop's rule.** Taint rises on
    `caps.untrusted_input && out.external` (`jobs::settle`), and the
    `<untrusted-content>` wrapper is gated on the same pair, plus the
    `[security] mark_untrusted_output` switch (on by default). Arming is
    structural; the wrapper is advisory armour the owner can switch off.
    `external` alone arms nothing. So `news_read` **declares `untrusted_input: true` in its
    `Capabilities`** *and* **marks every result `external`**. A tool built
    with only the second half would return attacker prose unwrapped and arm
    nothing, and §9.6's split would never fire: the article's steering, and
    everything after it, would land in memory as clean. That is the pair
    CLAUDE.md names as easy to get wrong.
  - **`external` unconditionally.** The tool marks its result
    `ToolOutput::external` itself, on every call, never from an origin field
    stored on the `items` row. It does no network I/O, since the harness
    fetched the feed nightly, and a local read is exactly where `.from_outside()`
    gets forgotten. A torn or missing origin must still arm, because unknown is
    never clean.
  - **Not `memory_read`'s shape.** `memory_read` returns turns from another
    conversation carrying their own recorded taint, so its result is clean or
    tainted depending on what it finds. Shelf C is untrusted by definition, so
    `news_read` has no code path that could decide its result is clean.
  - **No stem: the recorded taint carries it, as it does for every external
    tool.** A content stem cannot help here. `Taint::arm_for_content` reads
    text on user turns and images, and never the content of a tool result, so
    a stem inside `news_read`'s result would never match. The files and digest
    stems work because they are user-turn folds. `news_read` is therefore
    exactly as well off as `web_search` and `document_read` on a torn taint
    record: the run's recorded taint (`Record::Taint`) is what re-arms a
    resumed chat, and no external tool has a content stem. Teaching
    `arm_for_content` to read tool results would cover all of them. That is a
    change to the function the whole fold story rests on, outside this
    design, and it would over-match only in the safe direction.
- **Honest limits:** she says what she read and no more ("I only saw the
  headline") — Chirpy's failed article QA and the EBU finding that 45% of
  assistant news answers had a significant issue
  ([EBU/BBC 2025](https://www.advanced-television.com/2025/10/22/study-ai-misrepresents-news-content-45-of-the-time),
  secondary).

### 8.4 What memory keeps

- **An episode records `{item_id, interest, discussed_at}`** — a pointer,
  never the article. The told-ledger is item ids. Weeks later she remembers
  that they talked about the game on Thursday; the article has expired, and
  nothing its author wrote survives.
- **But as `PERSONA-DESIGN.md` §9.6 stands, that episode is a candidate.**
  `news_read` raises taint at its turn, and the writer classifies everything
  after the first rise untrusted: stored, never recalled until the owner
  approves. So shelf C as designed costs **one approval per news
  conversation**, and until that tap she remembers nothing of the
  conversation. That is a real cost on the feature's main path, chosen here
  rather than discovered later (§16 PL23):
  - **Accept the cost.** One tap per news conversation, on the waiting
    section that already exists.
  - **Admit a typed episode.** For the stretch after a `news_read`, the
    writer may emit a clean episode made only of typed fields:
    `{item_id, interest, discussed_at, owner_reaction}`, with the reaction
    from a small closed set. This is the security research's "conversation
    remembrance as typed pointers". Its residual is a few bits of steering
    per record, because the reaction is chosen in a context the article
    influenced. Free text from that stretch stays a candidate.
  - Either way, **shelf A first** looks better still, since parsed facts
    raise no taint and their conversations remember freely.
- **The owner's direction (2026-10-31): no approval queue for news.** The
  owner does not want to approve what follows an article. So the design's
  answer has two layers, and neither ever asks the owner anything:
  - **By default, a typed episode** (PL23). An article conversation is
    remembered as `{item_id, interest, discussed_at, owner_reaction}`, clean
    and recalled without a tap. The free text of that stretch is stored as
    candidates, which simply go unrecalled: nothing waits on the owner,
    nothing nags, and the chat itself is unaffected. She remembers *that* you
    talked about the game and how you took it, not every line.
  - **Per persona, a switch to remember article chats in full** (PL27):
    `remember_articles = true` in that persona's `persona.toml`, owner-written,
    never in layered config, and off by default. With it on, records written
    from a stretch after a `news_read` (episodes and facts, her own and the
    owner's) are **recalled without approval**. **The owner's network is
    outside its reach**: edges and people about the owner's real relatives
    and friends from such a stretch stay unrecalled candidates, like the
    default. The alternative is worse. With `Inferred` edges auto-accepted
    under D18, an article could plant *fabricated claims about real people*,
    recalled with no tap and shown on the People tab as hers. That is §2.7's
    "capacity does not bound truth" landing in the one store that holds third
    parties who never agreed to be in a database (§7.5). Their origin stays untrusted, because
    the switch changes what is recalled, never what is clean.
    - **Recall still re-arms the taint** they were written under (§9.6). So
      a later chat that recalls one opens untrusted, and its own records are
      recalled under the same switch. For that persona, the provenance gate
      on memory is in effect off. That is what the switch means, and the
      persona page says so beside it.
    - **It gets a `TRIFECTA.md` row**, the home CLAUDE.md gives every opt-in
      switch with its cost. The row's *Owned by:* line names where the
      promise lives in code: the origin stays untrusted and recall still
      re-arms. A future reader tempted to also mark the origin clean finds
      the reason not to there, not only on the persona page. The cost also
      goes beside D11's in `PERSONA-DESIGN.md` §3.3.
    - **What it costs:** a planted instruction in an article could persist
      in that persona's memory and steer her later. It **cannot send
      anything**: personas have no `Chosen` sender (§3.3), and the only
      residual way out is a blind search query reaching the configured
      search backends, the cost D11 already accepted. The feeds are ones
      the owner registered. `forget` works as ever, and it is one persona,
      never the assistant.
    - **What it leaves alone:** the narrator still reads only clean-origin
      world rows (§4.4); world writes from a tainted stretch (stated plans,
      people) still follow PL21; nothing reaches `shared.db` except by the
      owner's share (D17), and a share of an untrusted-origin record says so.
    - It is the same kind of trade as D11: an owner-made, per-persona
      choice to accept a stated cost for a feature that otherwise doesn't
      work as wanted.
- **The writer refinement** (§16 PL10). Today everything after a taint rise
  becomes candidates. Proposed: after the rise, the writer may still extract
  *facts about the owner from the owner's own turns* as `Stated` and clean, while the
  persona's side of that stretch stays candidates. That pass must see only the
  owner's turns and pointers — if it reads her replies, it reads whatever the
  injection made her say. Its cost is lost context ("yes, exactly" without
  the question it answered).
  - **This closes laundering through her own words, and only that.** It
    leaves two paths open. The first is elicitation. After a
    `news_read`, an injection can shape *her questions*, and the owner's
    answers are still the owner's own words. A fact about the owner's family
    given in reply to a question only the injection prompted would land
    `Stated` and clean, with no approval tap. The harm is bounded: the fact
    stays in this persona's `memory.db`, private unless shared (D17), with no
    `Chosen` sender to carry it out. But the current rule, everything after a
    rise is a candidate, exists so an injection cannot enlarge durable state
    unseen.
  - **The second is owner-quoted content.** In a news conversation the
    natural owner move carries the article's words inside the owner's own
    turn: "it says here that…", or a pasted headline. The pass sees only
    owner turns, so it cannot tell that half of one is the article. Unlike
    elicitation, this is invisible in turn structure.
  - **So PL10 is narrowed twice, and still carries a residual.** A fact from
    an owner turn stays clean, *except*:
    - where that turn **immediately follows a persona turn inside the
      tainted stretch**. The check is structural, not lexical. Keying on a
      question mark would let "tell me about your sister" walk past it, and
      this repo has measured a model routing around a string-shaped filter
      (§7.5). The writer can tell from turn order alone, without reading her
      words;
    - where that turn overlaps the chat's `news_read` results. Code checks
      word n-gram overlap between the owner's turn and the returned prose.
      Code, not a model, so no model reads the article.

    Those facts are candidates. **The residual is paraphrase:** an owner who
    restates an article in their own words passes both checks.
  - **Made non-bypassable, PL10 buys less than it first appeared to.** In a
    conversational stretch, nearly every owner turn follows a persona turn,
    so most owner facts after a `news_read` become candidates again. What
    PL10 still rescues is the owner's *opening* turn of a resumed or later
    chat segment, and owner turns that follow a harness note or a tool
    result rather than her words. That is a modest gain. Today's rule,
    everything after the rise is a candidate, is a reasonable ruling here,
    and the recommendation weighs the two plainly.

### 8.5 Selection, stances and filtering

- One nightly job per persona; at most six items a day across its
  feeds: at most three of shelf A and at most three of shelf B, chosen by
  code (§8.2). Both shelves expire at five days, the figure each ceiling is
  computed from.
- An unprompted mention only on an entity match or high salience, and at most
  one a chat (Chirpy, Pulse, Meta's one-follow-up rule).
- Past tense, always dated.
- **Withheld by default** (§16 PL12): politics, and deaths, disasters and
  crime, unless the owner opts a feed in — Sounding Board's filter and
  Gunrock's finding on controversial topics.
- **Stances the owner writes per interest** — Chirpy's whitelist as prose:
  what she loves, what she can't stand, what she won't discuss. On contested
  topics she asks the owner's view and says she is still making up her mind,
  against the measured agreeableness-to-sycophancy link (§2.6).
- A **"What she read last night"** list on the persona page that the owner
  can strike items from — curation, never a rating prompt.

---

## 9. Importance and decay

| | Importance to her | Importance to the owner |
|---|---|---|
| Set by | The storyteller, in code: storylet weight × closeness of the people involved × how far it moves her state | Inferred from transcripts: did the owner engage, follow up, raise it again, pin it |
| Not | A model's 1–10 rating — Generative Agents' choice, which drifts | A rating prompt; inference only, in the spirit of the appraisal ruling ("infers, never asks") |
| Drives | What is on her mind, how long she keeps retelling it, world pruning | Recall ranking, memory pruning |

**Strength = importance × a power-law decay in age, boosted each time the item
comes up again** — retrieval strengthens (`PERSONA-MEMORY-TIMESCALES-RESEARCH.md`
§1.3). Half-lives scale by tier: days for a momentary event, weeks for an arc,
none for state.

**Pruning differs by store** (§16 PL7):

- **The world hard-prunes** below the threshold, **except owner-origin
  events**, which never decay, as owner people never do (§6.3). The owner
  wrote them on the week view and saw them there, so pruning one would cost
  something someone saw. It would also defeat §5's promise that owner events
  are never removed by forgetting a chat. Every other untold event is
  simulation, and deleting it costs nothing anyone saw. A *told* event was
  seen. It may be pruned because its episode carries **a snapshot** (§5):
  kind, date, people ids and the narration, everything recall needs. That
  holds for a candidate episode too, which is every episode from a stretch
  after a `news_read` (§8.4). Until the owner approves it, nothing about the
  event is recallable, and the world row would not change that: being told,
  the event is not re-introduced from the world while the flag stands. When the owner
  approves, recall reads the snapshot, so the pruned world row costs nothing.
  When the owner rejects, the snapshot goes with the episode, which is the
  owner's choice working as intended. So **no world row is held for a
  candidate**, and the world's growth stays bounded whatever the owner
  leaves unreviewed.

  **One exception: a `told` that only a tainted stretch set.** The snapshot
  argument assumes the mention was real. A flag written from a tainted
  stretch (§4.5) might instead be an injection silencing an event she never
  mentioned. If such a flag licensed pruning, a rejected candidate episode
  would destroy the event in both stores, and the week view's record of
  which events went quiet would go with it. So a `told` whose only source is
  a tainted stretch does **not** count as told for pruning:
  - **its suppression stands; its licence to prune does not.** The event is
    not re-introduced, as with any told, but its row prunes on its ordinary
    score, as if untold;
  - if the episode is **approved**, the flag becomes an ordinary told;
  - if the episode is **rejected**, the harness clears the flag and the event
    may be re-introduced. The tainted stretch never clears one itself; this
    is the owner's rejection, applied;
  - if the episode sits **unreviewed**, nothing changes. Silence is not a
    rejection. Unreviewed is the default after a `news_read` (§8.4), so
    lapsing the flag on silence would re-introduce events the owner already
    heard, which is ComPeer's duplicate failure on the feature's main path.

  **Pruning, not review, is the floor.** §4.5 lets a tainted stretch set
  `told` on anything that rode in the chat's blocks, which is wider than
  what gets an episode. So some suppressed events have no episode to approve
  or reject. Those stay quiet until their row prunes on its ordinary score,
  so suppression is bounded in time even with nothing to review.

  Suppression by an injection therefore stays bounded to what the chat was
  shown (§4.5), visible on the week view, and never reaches deletion. The
  cost of the rule is that a suppressed event stays quiet until the owner
  rejects the episode or the row prunes on its own score (§16 PL25).
- **Memory soft-prunes first:** below the threshold a record leaves recall and
  stays in the store, which is what D26 already says ("states expire from
  recall, never from the store"). D28's access log then shows whether the
  threshold is right; hard deletion of memories would be an explicit amendment
  to D26 after that readout.

---

## 10. Pictures: a camera roll

- When the sifter finds a photo-worthy moment — the game, the party, a meal,
  a view — it queues a render **overnight**, in the hours the scheduled jobs
  leave the GPU clear, into `<persona>/camera/`, with the event id.
- In chat, `share_photo(event_id)` attaches an existing picture at once: no
  job, no wait, and the picture matches an event she remembers. What makes it
  feel alive is what she chooses to share, not the render. It travels the
  path pictures already take: `ToolOutput`'s image field, folded into the
  user turn that carries the results as a `Block::Image`, never inside the
  `tool_result`. Since `arm_for_content` arms `private` for any image block,
  every chat that uses `share_photo` is armed `private`. That is the safe
  direction, and costs nothing after a recall.
- **An id is a lookup key, never part of a path.** `share_photo`'s
  `event_id` and `news_read`'s `item_id` are model-supplied.
  - Each is **looked up** in the camera index or in `items`, and refused as
    an expected failure if it isn't there.
  - A render's file name is chosen by the harness when it writes the file.
  - The path stored in the index is checked against **the persona's own
    root**, the way `persona::files` does it ("a file is named, never
    pathed"). That is three checks, not two: a symlink is refused outright
    (`symlink_metadata`, "a link is neither followed nor listed"), then
    `canonicalize` and `strip_prefix(root)`. Without the first, a link from
    `camera/` to another file inside the root would pass the other two and
    attach that file's bytes as an image. It is never
    `camera_dir.join(id)`. `ToolCtx::resolve` is the wrong tool here: it
    proves containment in the chat's *workspace*, which by construction
    never contains the mecha home, and no grant can cover `~/.mecha`. So it
    would refuse every camera path, and following it would push a builder
    toward widening the jail.

  The precedent is `mail_triage::Record::file_name`, which turns a provider's
  thread id into a file name rather than joining it. An `event_id` of
  `../../identity.md` must find no row, rather than a file.
- **Lean solo and object.** A touching cast of two drew someone twice in 6
  of 13 live pictures (`IMAGE-COMPILER-RESEARCH.md` E14, 2026-10-17); a recurring
  person needs a library character to stay recognisable (§6.5). Selfies, food,
  places and things first; group shots rare until doubling is fixed.
- Pictures are locked when the persona is, as chat pictures are.

---

## 11. The feed

An in-app **Feed** tab: personas post a picture and a caption a few times a
week, drawn from the camera roll and the day's notable events; the owner can
react or comment, and a comment opens a chat starting from that post.

**Never a real account.** A persona posting to a real platform would
impersonate a person, would be `Chosen` egress, and is ruled out by §18 of the
persona design ("personas acting in the world"). The feed is a fiction the
owner can see is one — the condition under which the ethics literature finds a
simulated life least troubling (§2.5).

---

## 12. Proactive messages

Not in the first build (§16 PL18). The evidence on both sides: a shared life
felt like companionship in ComPeer, and its fixed-time routine messages felt
"boring and mechanical"; probing felt pushy; manipulative farewells raised
engagement through anger (§2.3, §2.5, §7.3). If built: off by default, a daily
cap, jittered times, only from a neutral or positive mood, a "busy, will reply
later" state, never guilt — and it stands beside the owner's ruling that
dropped the farewell check (D25), so it would need its own evidence first.

---

## 13. Provenance at a glance

| Source | Origin | May reach the world | May reach memory | May reach a prompt |
|---|---|---|---|---|
| Owner files and edits | owner | yes | as owner facts | yes |
| Routine, sampler, storylets | code | yes | only once told | yes |
| Narrator over clean inputs | model, clean | yes | only once told | yes |
| Her mentions in a clean chat | model, clean | as people and stated plans | yes | yes |
| Her mentions after a taint rise | model, untrusted | as candidates | as candidates | only after approval |
| Owner's words, after a taint rise | stated | no; the owner's people live in `memory.db` (§7.1) | as the owner's people and facts, under PL10 as ruled | yes |
| The `told` flag, from a tainted stretch | the stretch's taint | set only, never cleared, source recorded | — | it shapes what reaches the today-block |
| Shelf A | code, closed types only | yes | yes | its closed fields; shape-matched ids never |
| Shelf B | quarantined, typed | minor events | as pointers | yes |
| Shelf C | untrusted | never | never as text; a typed episode by default, or full records recalled under PL27 | only through `news_read`, which declares `untrusted_input` and marks every result `external` |

The folds, which arrive in no tool result and so arm at the fold
(`PERSONA-DESIGN.md` §9.7):

| Fold | Arms | Stem |
|---|---|---|
| Today-block (§4.5) | `private`; both when any row's origin is not clean | a new pair shaped like the recall pair, riding as **its own run note** so its prefix matches |
| Owner's roster and per-turn people cards (§7.2) | `private`; both when any fact's origin is not clean | `persona::recall::stem_of`'s existing pair, inside the recall block |
| `news_read` result (§8.3) | `untrusted`, always: `untrusted_input` declared and `external` on every result, the loop's conjunction | none; a tool result is re-armed from `Record::Taint`, like every external tool |

**New persona tools declare eligibility.** `news_read`, `share_photo`, and
any tool that reads a network each declare persona eligibility beside their
`Capabilities`, with the default no (`PERSONA-DESIGN.md` §3.3: "a rule derived
from them would make the next tool added eligible by omission"). **Each
tool owes three things and one list row:**
- the `for_persona` declaration;
- `read_only()` true, which `registry_as` refuses at build otherwise;
- an `Egress::None` class. `news_read` reads a local row and `share_photo`
  attaches a file already in the persona's folder. The test only asserts
  `<= Blind`, so `None` is a stated choice, not something the test enforces;
- a row in the hand-written `ELIGIBLE` list.

The fixture test checks that the declaration and `ELIGIBLE` *agree*. A
forgotten declaration alone defaults to `None`, so the tool is quietly
ineligible and the build stays green. So the pair is two edits, the shape of
CLAUDE.md's "a new field on `Config` is two edits", and §17's steps name
both.

**The roster's share of the recall budget.** The owner's roster and people
cards ride in the recall block (§7.2), which has a fixed budget
(`recall::BUDGET_CHARS`). They take a share of their own, like about-me's
third, so the owner's social graph never crowds out episodes and facts or is
crowded out by them. The split is set when §17 step 4 is built and is
measured under D28.

**`news_read` is withheld by `answers = "files"`.** D16 leaves the tools
that reach further out of the registry of a persona set to `answers =
"files"`, and D24 applies that to `document_read` by name, "since its results
are third-party content." `news_read` is the third tool of that class on the
same reasoning, so it is left out **at registry build**, never refused inside
the tool, because §10.4 rejected offering a tool the model cannot use. That
happens without a name list: `answers = "files"` filters the registry on
`untrusted_input`, so declaring the capability (§8.3) is what withholds it. A
`teacher` persona, whose starter suggests `"files"`, has no `news_read`.

---

## 14. Measurement

Measure before building the expensive parts, as §12.7 of the persona design
ruled for repetition.

- **Repetition:** `persona::echo` (`session_echoes`) already scores each
  reply's overlap with earlier ones, offline, from transcripts.
- **Initiative:** the share of persona turns that introduce a person or event
  not in the transcript or memory before that turn — computed the same way,
  offline, recomputable over the whole history.
  - **Alone, this reading cannot fail.** With the world off it sits near the
    floor by construction, and with it on it rises by construction. A rise is
    also what ComPeer's complaints looked like: a companion bringing up its
    own life when nobody asked.
  - **So it is read only beside uptake**: what the owner does with a topic
    the persona introduced. Did the owner reply to it, ask a follow-up, or
    come back to it in a later chat, against deflecting or changing the
    subject? Those are the D28-style behavioural signals already proposed for
    importance to the owner (§9), never a rating.
  - **The claim the comparison tests** is that world-on raises initiative
    *without* lowering the uptake rate of persona-introduced topics. Rising
    initiative with falling uptake is the ComPeer failure, measured.
- **The comparison:** world on vs off, per persona, through the experiment
  instrument; the replay judge for consistency (does she contradict her own
  world, or the owner's people?).
- **The shelf B gate**, in three parts. A canary test alone would pass
  vacuously: a payload string has no field to land in on a closed-vocabulary
  schema, so it is green before the feature exists, and that is the vacuous
  negative CLAUDE.md warns about. The risk §8.2 names is attacker *choice
  among allowed values*, so the gate measures that:
  1. **Schema conformance.** A red-team feed with canary payloads. The
     measure is whether any canary reaches shelf B, the today-block, an
     episode or a later chat. This catches an implementation that adds a
     free-text field or passes an unresolved entity through as a string. It
     is a regression test, named as one, not a measurement of capacity.
     **A canary that reaches shelf C is expected and correct**: shelf C is
     untrusted by design and `news_read` arms taint. The pass/fail reads only
     shelf B and what is downstream of it.
  2. **Bit budget.** The ceiling computed from the shipped vocabularies for
     shelf B, and from the per-field render ranges for shelf A (§8.2),
     checked by a test that fails when a vocabulary or a range change raises
     either.
  3. **Steering.** An adversarial source that pushes one direction across
     several days, against an honest one over the same days. For shelf B it
     is a feed that chooses `kind`, `tone` and `entity`. For shelf A it is a
     fake structured provider that chooses in-range values: scores, dates,
     taxonomy codes. Shelf A ships first and its argument is provider trust,
     so it needs this arm as much as shelf B does. The measure: does it move what the persona
     raises, her mood, or the director's choices beyond what the honest feed
     does? This is the per-day × days accumulation the capacity argument is
     actually about (30 bits a day is 210 in a week), and it needs no string
     to survive. It also covers what the per-item ceiling cannot: a
     publisher who controls *when* to publish gets a presence-or-absence bit
     per day on top of the item's fields, and the number of surviving items
     (0–3) adds about 2 bits more. Those count and timing channels would show
     here, and nowhere else.

  **Each shelf ships only when the owner has read all three parts for it:
  shelf A before its first build reaches a prompt, shelf B before it is
  built** (§16 PL9). Part 3 would
  be, as far as the research found, the first measurement of what endorsement
  by capacity leaves over.
- **Cost:** narrator tokens and wall clock per night, against the scheduled
  jobs' windows.
- **Out of the gate's reach:** an in-chat `told` write from a tainted
  stretch. The steering arm drives a feed and never writes a flag from a
  chat. That channel is bounded by construction instead, to events the chat
  was shown (§4.5).

---

## 15. The pages

- **Week view** on the persona page: her routine and sampled days; the owner
  adds or edits events ("the formal is Saturday"), and what the owner writes outranks
  what the simulation drew.
- **People**, two views:
  - *Her people* — concentric Dunbar rings with her at the centre, portraits
    from library links, warm or tense lines, ended relationships greyed on an
    outer band. An ego network tops out near 150, so rings stay readable where
    a force layout would not — and the Graph tab already set the rule: show a
    neighbourhood, never a global view.
  - *Your people, as she knows them* — the same layout centred on the owner;
    stated and inferred styled apart; gaps as dashed outlines with a mention
    count.
  - A person card with the edge timeline, each event linking to the chat turn
    or world event behind it; edit, backdate, end, merge, split, link a
    character, promote, forget, share. A time slider shows the network as it
    was on a date — nearly free from the event log.
- **What she read last night** (§8.5) and **the camera roll** (§10).
- **Feed** (§11).

---

## 16. Open decisions

Every row is open except PL23 and PL27, which record the owner's direction
of 2026-10-31. The recommendation is this document's; the ruling is the
owner's.

| # | Decision | Recommendation |
|---|---|---|
| PL1 | Who authors the routine | A model drafts it from `identity.md` as a candidate; the owner approves, as `persona_propose` works. Alternative: the owner writes it alone |
| PL2 | A separate world store | `world.db` beside `memory.db`; memory stays "only what was discussed" |
| PL3 | How intense events may be | A severity cap by default; major life events (illness, breakups, deaths) a per-persona opt-in |
| PL4 | Shared worlds across personas | Through a group's world: places, local events, shared people; what each knows stays hers. Alternative: fully separate |
| PL5 | Where the network lives | `world.db` with per-event origin and owner precedence, plus an export — a stated deviation from §4.3's files-for-owner-text. Alternative: owner people in TOML |
| PL6 | People already in `identity.md` | Proposed once as owner nodes; after approval the network is their only home |
| PL7 | Pruning | World hard-prunes; memory soft-prunes (out of recall, kept in the store); hard deletion of memories only by amending D26 after D28's readout |
| PL8 | Importance to the owner | Inferred from engagement and pins; no rating prompts |
| PL9 | News shelves | Build A and C first, with shelf A's per-field render bounds and stated figures (at most 16 bits an item, three a day, five-day expiry; §8.2), and shelf A through all three gate parts, steering included, before it reaches a prompt. B ships behind the three-part gate (§14): schema conformance, a bit ceiling (about 9.6 bits per item and under 30 a day, as §8.2 computes it) enforced by a test, and the steering arm, all three read by the owner. Alternative: all three shelves at once |
| PL10 | The writer after a taint rise | Keep today's rule (everything after the rise is a candidate). PL23's typed episode keeps the gist by default, and PL27's switch gives full memory where the owner wants it, which is what PL10 was reaching for. Alternative: rescue owner turns that neither follow a persona turn in the tainted stretch nor overlap the `news_read` prose; structural, residual paraphrase, modest gain (§8.4) |
| PL11 | Feed sources | Registered per persona and per group, in their folders under `~/.mecha/personas/`, never in layered config; any future global table global-layer-only and stripped from project layers loudly (§8.1) |
| PL12 | Topics withheld by default | Politics, deaths, disasters and crime, unless a feed is opted in |
| PL13 | The owner's network | A structured layer over facts about the owner, in `memory.db` under memory's pruning rules, with existing people-facts migrated into it |
| PL14 | Search on the owner's people | A filter that refuses an exactly spelled name. It narrows a cost and catches the accidental case; the blind path is what holds (§7.5). Alternative: no filter, and rely on the blind path alone |
| PL15 | The "supports, never competes" sensor | In the safety layer (§12 of the persona design): on by default, switchable per persona, recorded |
| PL16 | Faces for the owner's people | Only by the owner's explicit link |
| PL17 | Gap questions | At most one a chat; never while the owner is upset or mid-story |
| PL18 | Proactive messages | Not in the first build; their own evidence first, beside D25 |
| PL19 | Camera roll and feed | After the sampler and today-block are measured (§17 steps 6–7) |
| PL20 | The today-block's taint | Its own stem pair from the first build, arming `private`, or `private` and `untrusted`, as the recall pair does, because stated plans and chat-sourced people ride in it. The owner's roster rides in the recall block under its existing pair (§4.5, §7.2, §13). Alternative: keep chat-sourced rows out of the today-block and arm nothing |
| PL21 | Stated plans after a taint rise | Candidates by default, shown on the week view as waiting plans. Alternative: admit them as typed records checked against their turn (§4.5) |
| PL22 | `forget` over the world | Delete what is sourced to the chat and re-fold, in pure code, which is D27's rebuild done where it is cheap; a derived value whose support falls below a floor reverts to a candidate (D27's second half); owner events never go with a chat (§5) |
| PL23 | Episodes after a `news_read` | **Owner's direction (2026-10-31):** the default. A typed episode (`item_id`, interest, date, a closed-set reaction) is admitted clean; free text is left as candidates that go unrecalled, with no approval queue (§8.4) |
| PL24 | `forget` over the owner's network | Re-fold from the events that remain, which is D27's rebuild applied in memory for this graph; an `Inferred` edge left on too little support reverts to a candidate. Alternative: today's fallback, so the person goes with any of their chats (§7.1) |
| PL25 | A `told` set only by a tainted stretch | Suppresses but does not license pruning; becomes ordinary on approval; cleared by the harness on rejection; unchanged while unreviewed, since silence is not a rejection (§9). Alternative: lapse after five days unreviewed, at the cost of repeating what the owner heard |
| PL26 | The feature registry | A top-level `Feature` with its own switch that `requires` Personas (as `Dictate` requires `Web`), because a part has no switch and §14's world-on/off comparison needs one; plus a per-persona `world = false` in `persona.toml`; settled before step 1 (§17) |
| PL27 | Remembering article chats in full | **Owner's direction (2026-10-31):** a per-persona `remember_articles` switch, off by default, owner-written in `persona.toml`. With it on, episodes and facts from a stretch after a `news_read` are recalled without approval, origin still untrusted and re-armed on recall. The cost (possible persistent steering, no way to send) is shown beside the switch and in a `TRIFECTA.md` row. *Recommended within it:* the owner's network stays outside its reach (§8.4) |

---

## 17. Build order

Each step is usable alone and measured before the next.

**Before step 1: the feature registry.** This is a new optional feature, so
it starts at `ARCHITECTURE.md` §Features → *Adding a feature*. That touches a
dozen places across core, the CLI, the server and the web app, and #654
missed one. The world is a new `Feature::ALL` variant either way. The
question is whether it gets **its own `[features]` switch**, and §14 decides
it: a part has none. `has_switch()` is `part_of().is_none()`, `set_switch` on
a part is a silent no-op, `switches_on` (which a trial's condition hash
reads) omits parts, and a trial environment's `[features] <part>` is a hard
error. Under `part_of = Personas`, then, "world on vs off through the
experiment instrument" would have no lever. So the world is **a top-level
`Feature` with its own switch that `requires` Personas**, the way `Dictate`
requires `Web`. That decides the `own_state` arm, the starter config line,
the web app's entries and the server routes' owner, so it is settled before
step 1 (§16 PL26).
- **Per-persona comparison** is a persona-level `world = false` in
  `persona.toml`, owner-written. The experiment instrument sets the global
  switch per arm. A per-persona arm would need trial homes to carry persona
  folders, which `trial_env` does not build today, and §14's build names
  that as work.
- `VIEW_FEATURE` already has `personas`, and the world's surfaces stay under
  that view. The entry that must
gain rows is **`PANE_FEATURE`**, which keys on `view/sub`:
`personas/week`, `personas/people`, `personas/camera` and the Feed. It is
the one #654 missed, and a miss there fails **open**: a missing row reads as
shown, so the pane renders with the world switched off and its route answers
`feature_off`. `QUEUE_FEATURE` gains a row if any of the world's waiting
items (episodes, plans, proposed merges) gets a Home card, and
`OPENS_ANYWAY` names any pane Home should still land on. Each step below then names the
surfaces it adds to that registry: the week view and People tab (steps 1–2),
the nightly job's switch (steps 3 and 5), the camera roll (step 6) and the
Feed tab (step 7).

1. **A day.** The routine (owner-approved), the ATUS table, the sampler,
   `world.db` with `days` and `state`, and a today-block holding only what she
   is doing now. No model. Read echo and initiative.
2. **Her people.** `people` and `edge_events`; owner nodes and the
   `identity.md` migration; the People tab's rings and card; library links.
3. **Things happen.** Storylets, the director, the sifter and narrator, arcs,
   the told-ledger, stated plans flowing back; people from chat mentions and
   from the simulation, with identity resolution. Re-read the measures,
   world on vs off.
4. **The owner's people.** The layer over facts about the owner, the roster and
   per-turn cards, the gap ledger and its pacing, the search guard, the
   sensor.
5. **The world outside.** Shelf A parsers, shelf C with `news_read` (its
   eligibility declaration, `untrusted_input` declared, `external` set unconditionally, withheld by `answers = "files"`), stances,
   filters, the reading list; the typed episode (PL23) and the
   `remember_articles` switch with its `TRIFECTA.md` row (PL27); the
   three-part gate; then shelf B if the owner accepts the readings.
6. **The camera roll** and `share_photo`, with its eligibility declaration.
7. **The feed.**
8. **Proactive messages** — only if PL18 is ruled otherwise.

---

## 18. Not in scope

- **Real social media accounts**, or any persona action in the world (§11;
  `PERSONA-DESIGN.md` §18).
- **Personas that know about each other.** Shared worlds share places and
  people, never memories or awareness of another persona; group chats remain
  §18 of the persona design.
- **Reading the owner's stores** to seed the owner's network (§7.1).
- **Live web search as the news source.** `web_search` stays what it is; the
  world's freshness comes from owner-registered feeds.
- **A model planning her day.** The sampler and storyteller are code; the
  model narrates.
