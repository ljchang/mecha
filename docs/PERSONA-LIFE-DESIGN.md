# Persona lives — design

> **Status (2026-10-31):** designed; §17's registry step and the `[life]`
> switches of step 1 are built (`Feature::World`, `persona::Life`), and so
> is the routine's file with its activity set and owner approval
> (`persona::routine`, `persona::activity`; the model drafter is not built
> yet), and the ATUS table (`persona::atus`, §4.2); the rest of step 1 is
> next. **Every decision in §16
> is ruled** by the owner, the same day: PL1–PL22, PL24–PL26, PL29 and PL30 as rulings,
> and PL23, PL27 and PL28 as the owner's direction (no approval queue for
> news; every part opt-in per persona). §1 is the owner's requests, in the order they were made; §2 is
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
| Is any of this on by default? | No. Every part (world, her people, the owner's people, news, camera, feed) is a per-persona switch, off by default; a persona with none on is unchanged | §3.1 |
| Does she message first? | Only if the owner turns it on for her (off by default): at most one a day, at irregular times, only from a neutral or positive mood, never guilt, and she stops if two go unanswered | §12 |
| How do we know it works? | `persona::echo` already measures repetition; an initiative reading measures who brings new things; a three-part gate (schema conformance, a stated bit ceiling, a steering arm) decides each news shelf, the parsed-facts shelf before it reaches a prompt and the typed-gist shelf before it is built | §14 |

---

## 1. The owner's requests (2026-10-30 and 2026-10-31)

In the order they were made, paraphrased closely. Each is answered by a
section; §16 records which are ruled.

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
- **O12 — Every part is optional, per persona.** The world model, pictures
  and the rest are optional features for each persona; some personas will
  use none of them (PL28, §3.1).

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

### 3.1 Every part is opt-in, per persona (owner's direction, 2026-10-31)

Nothing in this design reaches a persona the owner has not turned it on
for. Each part is a switch in that persona's `persona.toml`, **off by
default**. A persona with none of them on is exactly the persona of today:
no world, no today-block, no new tools, no nightly work.

```toml
[life]                     # owner-written; outside persona_propose's schema
world     = false          # her simulated day, events, arcs, the today-block (§4–§5)
people    = false          # her own network (§6); needs world
my_people = false          # her model of the owner's network, and gap questions (§7); needs [memory] on, user_facts != "off"
news      = false          # interests and feeds, shelves A/B/C, news_read (§8); needs world
camera    = false          # the camera roll and share_photo (§10); needs world
feed      = false          # the in-app feed (§11); needs camera
proactive = false          # PL18: she may message first, gated (§12); needs world
remember_articles = false  # PL27; needs news
intensity = "capped"       # PL3: "capped" (default) or "full" (§4.3)
name_search_filter = false # PL14: refuse web searches naming the owner's people (§7.5); needs my_people

[safety]                   # PERSONA-DESIGN §12's table: on by default, each switchable
relationships = true       # PL15: the "supports, never competes" sensor (§7.5)

[life.decay]               # PL7: the decay and pruning knobs (§9); defaults shown
event_half_life_days     = 4     # a momentary world event
arc_half_life_days       = 21    # an ongoing situation
retell_boost             = 1.5   # strength multiplier each time it comes up again
world_prune_below        = 0.05  # world rows below this strength are deleted

[memory]                   # memory's own table (PERSONA-DESIGN §4.3), not [life]
recall_below             = 0      # PL7: memory below this strength leaves recall; 0 = off
```

`[memory] recall_below` lives in memory's table because memory exists for
every persona, with or without a world. Its default is **off**, so a persona
with no `[life]` part on is exactly the persona of today, and no existing
persona's recall changes on upgrade. That also matches D28: measure memory
in use, and read the report before any ranking change. The owner sets a
threshold per persona once the access log says where it should sit. The
`[life.decay]` knobs govern only the world.

- **A switch decides what exists, not what is refused.** A tool whose
  switch is off is never built into that persona's registry: `news_read`
  without `news`, `share_photo` without `camera`. That follows §10.4's rule
  against offering a tool the model cannot use, and it keeps the persona's
  tool list, the front of its cached prefix, stable. No nightly job runs for
  a persona with `world` off, and no block folds into its chats.
- **Dependencies are refused by name, never silently satisfied.** `camera`
  without `world` is a load error naming the missing switch, the way an
  unknown key is. It never quietly turns `world` on.
- **`my_people` needs no world, but it needs memory of the owner.** It is
  memory of the owner, not of her world (§7.1), so a persona can learn the
  owner's people without a simulated life of its own. But it is built over
  the facts about the owner, and `[memory] user_facts = "off"` stops the
  writer recording those (`persona::writer`'s gate on `Table::User` and
  `Table::Inferred`). With memory off there is also no recall block to carry
  the roster, the cards and the gap note, so nothing arms `private`. So
  `my_people` **requires memory on and `user_facts` not `"off"`, refused by
  name** like any other dependency. The gap ledger and any people tables sit
  behind the same gate, so a persona whose owner turned off memory of them
  never stores the owner's relatives by another route.
- **Two rules, by what a setting does.**
  - **Switches that decide which tools, blocks and runs exist** (`world`,
    `people`, `my_people`, `news`, `camera`, `feed`, `proactive`) are fixed
    per persona
    version. A chat pins the version it started with (`PERSONA-DESIGN.md`
    §8.1), because the tool list is the front of the cached prefix, so
    turning one on applies to new chats. `proactive` is read by the
    scheduler, not a chat, so it takes effect at the next scheduled check.
  - **Behaviour settings** (`remember_articles`, `intensity`, the decay
    knobs, `name_search_filter`) are read live, at each use, as the memory switches already are
    (`setup::persona_agent`: "turning memory off reaches the chat at its
    next call"). `remember_articles` in particular is the one an owner may
    want to revoke in a hurry, and turning it off stops the next recall.
  - **But `remember_articles` never reaches backwards.** Read live, it would
    also work in the other direction: switching it on after weeks of `news`
    would admit every candidate already stored from past article chats, a
    backlog nobody reviewed. So the mark is keyed to **when the chat
    happened, not when the writer runs**. The writer can wait nights for its
    model (§4.4), so a live read at write time would still sweep up a
    pending backlog. Instead, the switch's state is recorded in the session
    at the end of each stretch, and the writer marks a record only if the
    switch was on when that stretch was chatted. Recall admits a record only
    when it carries the mark *and* the switch is on now. So:
    - switching it **on** admits only what is *said* from then on;
    - switching it **off** stops recall at once;
    - a stretch chatted under the switch and written after it was revoked
      keeps its mark, but is not recalled while the switch is off.
  - **A live read that fails falls toward off.** `[life.decay]` is a
    subtable of `[life]`, and a bad decay value is a load error (§9). So an
    owner's edit can leave the live `persona.toml` unreadable while the
    chat's pinned snapshot still parses with `remember_articles = true`. An
    unreadable live file then fails toward the safe side for each setting:
    no recall under `remember_articles`, `intensity` at `"capped"`,
    `name_search_filter` **on** (the one setting whose safe direction is on),
    and **no pruning of her own rows that night** (a shared node's test
    reads only the shipped defaults, §3.2, so her typo does not stall the
    group's), and **no persona-level decaying rows in
    that turn's today-block** (the fold reads `world_prune_below` too, §3.2,
    and neither threshold is safe to guess; shared rows fold on PL29's shipped
    default, which needs no file). Rows the threshold never reads still fold:
    owner-origin rows and `state`, which §9 exempts from decay, and `days`
    and `items`, which are pruned on a fixed age (§5). Events, arcs and
    `people` rows decay on the threshold (§9), so they are withheld. A day's companions
    who are decaying `people` rows are withheld with them and render as
    "someone", as a pruned id does (§6.3). So a typo empties the part of the block that
    needs the owner's numbers, not the whole of it. The decay knobs have no "off", and shipped defaults
    could be looser than what the owner wrote, which is the substitution §9
    refuses. So nothing is deleted on a read that failed. It never falls back to the pinned value or the last
    value read, and the persona's notes say the file could not be read. That
    is `memory_tools::live`'s pattern, where a `None` refuses rather than
    guessing, and it keeps the revoke-in-a-hurry promise true even when the
    revoking edit has a typo elsewhere.
- **The page shows them together**, each with a line on what it adds and,
  for `news` and `remember_articles`, what it costs.

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

### 3.2 Three levels of world (ruled 2026-10-31, PL4)

The world is hierarchical, on the same three levels the persona store
already uses for files and facts (`PERSONA-DESIGN.md` §4.5):

```
 everyone      seasons · holidays · the calendar · feeds registered for everyone
   │           (a game result is the same fact for every persona)
   ▼
 group         a town, a workplace, a campus: places, local events,
   │           a shared minor cast (the hospital, the coffee shop, a reorg)
   ▼
 persona       her own life: routine, days, her people, her arcs, her state
```

Each level is its own `world.db`: `~/.mecha/personas/world.db` for everyone,
`groups/<group>/world.db`, and `<persona>/world.db`.

- **A shared event exists once, and each persona experiences it her own
  way.** A snowstorm in a town is one group-level row. Each member persona
  whose sampler is affected gets her own derived event pointing at it: one is
  stuck on a double shift, another is home with a toddler and no power. The
  facts agree because there is one storm, and the experiences differ because
  the lives do.
- **Existence is not knowledge.** A shared event has a visibility: *public*
  (everyone in the group knows: weather, a festival) or *participants* (only
  personas the sampler placed there know: a staff meeting). Two personas can
  share a world without learning anything about each other, and personas
  still never know that another persona exists (`PERSONA-DESIGN.md` §8.1).
- **Nothing from a chat writes above the persona level.** The everyone and
  group worlds are written only by the owner, the simulation and feeds
  registered at that level. Those writers are not all clean: a feed's items
  keep their shelf's origin (§8.2), and shelf C's are untrusted wherever
  they are stored. What the levels rule out is a chat. Nothing from a chat
  moves up a level on its own, as with `shared.db`. That covers writes to
  rows that already exist: the `told` flag, the retelling boost and
  `known_to_owner` (§4.5) land on her own derived row (§3.2's first bullet)
  or her own person node, **never on the shared row**. A shared person
  (§6.5) is held by the members' nodes that point at it: the group's node
  is pruned only once no member's node is above the threshold, so a
  colleague two personas keep mentioning never decays out from under
  them. Whether the shared node goes is how a shared row decays, so PL29
  governs it: for that test each member's node is scored under the shipped
  defaults, never under the member's own `[life.decay]`, so one
  persona's `world_prune_below = 0` keeps her own node but cannot pin the
  shared one for the group. The other direction holds too: a member whose
  own knobs prune her node sooner than the defaults would does not release
  the shared one early. Her node leaves everything she reads on her own
  knobs, but its hold, the dates and retellings the score is computed from
  and no name or line, stays until the defaults would prune it as well, so
  decay alone never leaves the test without a row to score. A `forget` can:
  it removes the chat's contribution to the hold like any row (§5), and a
  hold built only from forgotten chats goes, as the owner's erasure should. A node of hers that outlives the
  shared one points at a pruned id, which renders as "someone" (§6.3). So a tainted chat
  with one persona cannot plant an event that reaches every member, nor
  silence one for them all: a tainted `told` stays within §4.5's bound,
  what was in front of her. (What `forget` removes, in the persona's store
  and the group's, is §5's.)
- **Several groups at once.** A persona can belong to a workplace and a town.
  Both groups' shared events flow into her life, and place names are unique
  within a group, so the two never collide.
- **Each level paces and decays on its own.** A group has one storyteller and
  one nightly narration pass shared by its members, rather than one per
  persona, which costs less model time. **Group and everyone-level nightly
  work runs only when at least one member persona has `world` on**, so §3.1's
  "no nightly job for a persona with `world` off" holds at every level. A persona's derived events decay
  under her own `[life.decay]`.
  - **A shared row that decays does so under the shipped defaults** (ruled,
    §16 PL29); §9's owner-origin and `state` exemptions hold at every level.
    (Seasons, holidays and a term calendar live in the routine file or a
    group's, §4.1, never in a `world.db`, so no decay reaches them.)
    Members may disagree, and a shared row is one row, so no one member's
    `[life.decay]` can govern it, even in a group of one. A level's own
    knobs, if wanted, arrive with the group world in step 3, in the owner's
    files and never in layered config.
  - **No pruner runs while no member has `world` on, and none is needed.**
    Strength is a function of age, computed when the pruner reads a row, so
    skipped nights lose nothing. **The fold computes strength too**, and
    folds no row below `world_prune_below` (the shipped default for a
    shared row, PL29), so a chat started the morning a member turns `world`
    back on, before that night's prune pass, folds nothing the pass would
    delete. The pass then removes what decayed meanwhile. Until then
    nothing machine-written lands in the level's `world.db`; the owner's
    edits still may.
  - **`items` are not a curve, and their expiry holds without a pruner.**
    Their five days are a fixed lifetime (§5). `news_read` refuses an item
    past its expiry whether or not the prune pass has run, and the
    today-block's shelf C lines carry only the night's arrivals, never the
    live shelf (§8.3), and no arrivals land on a night when no member has
    `world` on. So the day a member turns `world` and `news` back on cannot
    reopen a stale article by either path.
- **Feeds attach at any level.** A team's feed or local news is registered
  once at the group or everyone level (§8.1), and every member persona sees
  it.

The group-level storyteller, and the step that derives each persona's
experience of a shared event, arrive in §17 step 3 with storylets. Step 1
builds only the persona level.

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
(`persona::approve_as_shown` in `mecha-core/src/persona.rs`, the persona
side's twin of `imagelib::approve_as_shown`, and the function
`PERSONA-DESIGN.md` §4.4 names); nothing reaches a sampler until the owner
approves (§16 PL1). **The owner can edit at both ends** (ruled 2026-10-31):
the draft before approving it, and the approved routine at any time
afterwards, on the persona page or in an editor. An owner edit is an
owner-origin change, so the model never overwrites it, and a redraft
requested later comes back as a new candidate beside the owner's version,
never over it. A term calendar, holidays and seasons ride in the same
file or a group's.

### 4.2 The day sampler

A semi-Markov chain over ATUS tier-1/2 activities (§2.2), estimated once from
the public files for a handful of demographic cells and shipped as a table:
per-hour transition probabilities, an empirical duration distribution per
activity, and a who-was-present distribution from the Who file.

- **As built (`persona::atus`):** the 2023–2025 files, holidays left out,
  in six cells: a full-time college student, full-time employed without and
  with a household child under 13, part-time employed, not employed under
  65, and 65 or older. The table holds weighted counts, so how a thin cell
  backs off to the pool is the sampler's code. Durations are 21 quantiles
  per start band, leaving out each diary's first stretch, which began
  before the diary's 4am. Who was there is ten kinds of company, from
  partner to colleague to strangers, which casting maps onto her network's
  edge types. How a persona is placed in a cell is the sampler's step.

Each day:

1. Place the routine's anchors (each drawn against its probability).
2. Fill the gaps from the chain, conditioned on day type and cell.
3. Draw who was present for each episode from her network (§6), weighted by
   closeness and the Who file's distribution for that activity. **Only
   clean-origin people are cast**, by the sampler and by the director (§4.3).
   A person who entered her network from a tainted stretch is never drawn
   into a simulated day. A derived row's origin is the union of its inputs',
   so a clean `days` or `events` row never carries an untrusted person. The
   narrator also receives people as ids and closed fields only, so a
   person's description line never crosses into its prompt (§4.4, and §5's
   ids-not-names rule).

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
- A **severity cap, as a per-persona setting** (ruled, §16 PL3):
  `intensity = "capped"` by default, so Holmes–Rahe-scale events (a death, a
  divorce, a serious diagnosis) never fire; `"full"` opens them for that
  persona, still under the director's pacing. It is a closed set, so it
  follows the house rule for one. An **unknown value loads as `"capped"`**
  (`#[serde(other)]`, the narrowest variant) and is reported in the
  persona's notes; only an unknown *key* fails the load. That way a typo
  narrows a persona and never opens Holmes–Rahe events.
- **Most events involve other people** (§2.1): the director weights storylets
  with a cast member above solo ones.

### 4.4 The narrator

A sifter picks the day's one to three notable events — by importance to her
(§9), novelty against recent events, and the director's budget — and the
local model writes each up in a few sentences: what happened, who was there,
how it felt. Everything else stays a typed record nobody narrates. That is the
whole of the model's role: code decides what happened and when; the model
writes how it felt.

It runs nightly beside `persona memory write`, as a **one-shot with no
tools**, so an unattended nightly model run has nothing to call. The
narrator is not per chat,
but `writer::pick_model` already has the shape it needs: called with the
persona's pinned model (`PERSONA-DESIGN.md` §12.6) against the router's
resident one, it answers `Use` when they match or the router is empty and
`Wait` when another model is resident. The narrator calls it as-is. For a
persona with no pinned model that means the resident model narrates, so the
narrator's model is not always one the persona chats on. A group-level
narration pass (§3.2) has no one persona, so, like the group-level extractor
(§8.5), it asks with no pin and uses whatever is resident. Otherwise it waits for a later night, never
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
is typed values from **both** shelves, and it reads **only that night's
arrivals**, never the live shelf:
- shelf B, under 30 bits;
- shelf A, at most 48 bits, the larger channel, and clean by provider trust
  under render ranges rather than by capacity alone (§8.2).

That is **at most about 77 bits a night**, plus the publisher's placement
of shelf B's three arrivals among k interests, at most 3·log₂k bits, which
no vocabulary carries (§8.2). (Reading the live shelf over five
days would be about 385, which is why it reads arrivals.)
That influence can *select among* narrations the model would write anyway,
but it cannot *author* a string. That is a stronger argument than the model
resisting, and it survives §2.7's warning that a local writer should be
assumed weak. If shelf B ever gained a free-text field, or shelf A ever
stored an unparsed string (the case §8.2's "stored, never rendered" rule
exists to prevent), the narrator's output would stop being `ModelClean`. That is
one more reason §14's gate part 1 exists.

### 4.5 The teller: the today-block and the told-ledger

- **The today-block** rides in the message stream, as the recall block does
  (`PERSONA-DESIGN.md` §8.2) — never the system prompt, so it never costs the
  tools-and-system prefix that matters. It is not quite free: a per-turn
  note makes the last turn re-prefill once it is projected off the wire,
  and the today-block makes that two notes a turn beside the recall block. Under a fixed budget it holds: what she is doing now (the
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
    files, clean narration and shelves A and B. Shelf C's lines ride here
    too (§16 PL30), as the one named exception below. But stated plans and people she
    mentioned in a chat ride in it too (§13): a plan the two of them made is
    memory of a conversation with the owner, the case §9.7 arms `private` for.
    Arming is close to free here, since D11 already lifts the leak guard in
    persona chats and a persona's `web_search` is blind at every depth
    (`PERSONA-DESIGN.md` §3.3). The alternative, keeping chat-sourced rows out
    of the block and sending them through the recall block, splits one day
    across two folds for no gain.
  - **the untrusted stem arms `private` and `untrusted`** the moment any
    folded row's origin is not clean. Origin is per row (§5), and a row
    whose fields are all from code-fixed sets still arms it: a typed plan
    (PL21) has code's vocabulary and about 20 attacker-chosen bits of value.
    **The one exception is a shelf C line in the first build (PL30)**,
    before shelf B exists, which is not a row's fields but a line code
    composes: the interest from the owner's feed
    mapping and the harness-minted id, carrying no value the publisher
    wrote. Its presence, count and timing, and for a publisher reaching
    feeds under several interests which one a line shows (§8.3), are the
    residual §14's steering arm covers. Nothing else is exempt. Once shelf B ships, the id rides
    shelf B's interest line instead (§8.3), and that line folds by the
    origin of the clean half §8.2's single constructor emits, never the
    untrusted half beside it in the same `items` row (an `items` row
    carries one origin per shelf half, §5): clean by shelf
    B's capacity, with the untrusted variant below ready if its gate (§14,
    PL20) comes back dirty.
    Shelf B's line is clean by its own capacity, not by this exception.

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
  three predicates every persona fold is registered in:
  - `arm_for_content`, which the `Record::Notes` re-arm reaches through
    `arm_for_notes`;
  - `message::is_recorded_note`, which keeps a per-run note that somehow got
    stored off the wire, as the scene fold already relies on;
  - **`agent::is_harness_voice`**, which carries the longest-lived failure.
    The intervention miner cannot otherwise tell harness text in a user turn
    from the owner's own words, so an unregistered voice would be mined as
    the owner's correction into a learned rule riding in every future cached
    prefix.

  Nothing mines persona transcripts today, so the third is latent. But every
  existing persona fold is registered there, and that function's own rule is
  that "the party that knows a new voice exists is the one that adds it."
  §17 step 1 registers all three.
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
  - **A flag lands on her own row.** For a shared event, `told` marks her
    derived event, never the group's row (§3.2), so one persona's chat
    never decides what the others have left to say.
  - **The set is bounded by what the chat was shown.** A tainted stretch may
    set `told` only on events that rode in *that chat's* today-block or recall
    block, never on arbitrary world rows. So an injection that makes her
    "mention" things can silence at most what was in front of her, a handful
    of events per chat. This channel is outside the reach of §14's steering
    arm, which drives a feed, not a chat, and §14 says so. **The shown set has
    a recorded home**: each today-block and recall fold records the world
    event ids it carried, as a typed list beside the note in the session
    (alongside `Record::Notes`), so the writer checks a tainted `told`
    against ids, never against the note's rendered text. An id not in that
    chat's recorded set is refused.
  - **The same nightly read makes two other writes to existing rows, and a
    tainted stretch makes neither.**
    - **The retelling boost** raises a row's strength each time it comes up.
      That is the unsafe direction: an injection that made her "mention"
      chosen events or people would keep them above the prune threshold and
      at the top of the today-block indefinitely. So **a tainted stretch never
      boosts**; the safe direction for strength is down.
    - **`known_to_owner`** flips when she first mentions a person. Set from a
      tainted stretch, it would make her stop introducing someone she never
      actually named to the owner. So a tainted stretch never sets it either.
  - **Under `remember_articles`, the approve and reject branches are
    unreachable for an article chat.** Its episodes are recalled rather than
    queued, so nothing is reviewed. The exception keys on the `told`'s
    source, not on candidacy, so PL25's floor still bounds it: the row prunes
    on its ordinary score.
  - **On the main path, only one of §9's branches fires.** PL23 makes
    "unreviewed" the ordinary state of an article chat's candidates, so a
    tainted-only `told` normally just suppresses until its row prunes. The
    approve and reject branches are reachable from the Memories tab and
    matter less, so a builder implements the unreviewed branch first.
- **Stated plans flow back.** When she says in a chat that something will
  happen ("there's a game on Saturday"), the writer records it as a **stated
  plan**, and Saturday's simulated day contains it. Without this, chat and
  world disagree within a week.
  - **So do plans the owner states.** If the owner says "we've got the game
    on Saturday", that is the better-sourced plan, and leaving it out would
    bring the disagreement back from the other side. From a clean stretch it
    enters the world as an **owner-stated plan**:
    - it outranks the simulation;
    - it expires once its date passes, like any plan;
    - it goes with `forget` on its chat, being chat-sourced. Only owner
      *edits* on the week view are exempt from that.

    After a taint rise it follows hers (below): a waiting plan by default,
    or a typed plan with no approval when `remember_articles` is on. The week
    view is still there for plans nobody said aloud.
  - **These are not commitments in `guilt.rs`'s sense, deliberately.** That
    module's rule is "an expectation is a recorded commitment, never a claimed
    one": a model *saying* something creates no obligation, and its stores
    are all owner-visible queues. A stated plan is the opposite case, a
    model's claim written into her world. It shapes a simulation and owes
    nobody anything, so it is named differently to keep the two apart.
  - **After a taint rise, a stated plan is a candidate by default**, as §13
    and §6.2 already say for her mentions in a tainted stretch. It shows on
    the week view as a *waiting plan* until the owner approves it, so a
    news-following chat whose later plans never happen is visible rather than
    silent (ruled 2026-10-31, PL21).
  - **With `remember_articles` on, it enters as a typed plan, with no
    approval** (ruled the same day). The owner turned the switch on to avoid
    approvals, and a plan held out of the world would make what she remembers
    and what her world runs disagree. So under the switch, for stretches it
    covers (§8.4), the writer admits the plan as a typed record, never as
    text:
    - **a date**, within the next 90 days (about 6.5 bits);
    - **an activity** from a closed set fixed in code, at most 32 entries
      (5 bits);
    - **at most two people**, each an index into a **candidate list code
      supplies**: the clean-origin people whose names appear in that turn,
      matched against the roster by string, capped at 15 plus none (4 bits
      each, 8 bits for two). These are her network's people, or the owner's
      people for the owner's plans. The writer never ranges over the whole
      network, the way shelf B's entity field never ranges over the whole
      registry (§8.2).

    That is **at most about 20 bits a plan** (6.5 + 5 + 8), and at most three
    typed plans a day per persona, chosen by a seeded draw as typed episodes
    are (§8.4). The total is **at most about 60 bits a day**, the constant
    §14's bit test checks for plans.
    - **For an owner-stated plan, transparent endorsement does not hold,
      and that is stated.** The writer picking the people indices sees the
      article-steered turns *and* the owner's own, and the list is the
      owner's real people. So the choice is attacker × private data, the
      shape §8.2 rejects for shelf B and §8.4 flags for `owner_reaction`.
      It is acceptable for the same reason as there: with no `Chosen` sender
      (`PERSONA-DESIGN.md` §3.3), the 8 bits can tilt which real person her
      week pencils in, but cannot be read back out.

    Plans expire when their date passes. Each record is checked against the turn it was said in: the
    date and activity must both be stated there, or the plan is dropped and
    named in the persona's notes, never stored loosely. The record keeps its
    untrusted origin and the "from an article chat" badge, so the week view
    shows which plans came from such a stretch. Free text from that stretch
    still never enters the world. The residual is a planted plan, steered by
    an article, that her week then runs. That is bounded by the fields and
    the cap, visible by the badge, and removable on the week view or with
    `forget`.

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
| `people`, `edge_events` | Her network (§6) | Minor people decay out (§6.3); a node pointing at a shared person keeps only its hold, dates and retellings, past her own prune until the defaults would prune it (§3.2), and `forget` removes what a chat contributed to it like any row |
| `items` | Interest shelves A and B as typed rows; shelf C's prose (headline, summary, link) in an untrusted column `news_read` alone reads, never rendered into a block (§8) | Five days, all shelves; a feed's items go with the feed when the owner unregisters it |

**State vs events** is what makes aggressive pruning safe. "She changed jobs"
is state; the leaving-drinks evening is an event. Events decay, state is
updated — so decay removes texture, never a fact about her life. This mirrors
the tiers ruled in D26 (momentary, short-term state, ongoing situation, lasting
fact): an event is momentary, a mood is short-term state, an arc is an ongoing
situation, a state row is lasting.

**Every row carries an origin and a source** (owner edit, storylet, chat turn,
feed item), which is what `forget` on a chat works from. **An `items` row
carries one origin per shelf half:** clean for shelf A's typed columns,
by §8.2's closed parse, and for shelf B's, the clean half §8.2's single
constructor emits; untrusted for shelf C's prose column. A fold's per-row test reads the origin of the half it folds
from (§4.5), so a shelf B line beside a shelf C column stays clean.

**D27 is a rebuild that memory doesn't yet have, and the world can do it
cheaply.** D27 ruled "rebuild the fact from the chats that remain"; until that
re-derivation exists, a memory fact goes with any of its chats. The world must
not inherit that blunt fallback, because the network is a fold over events
from several sources (§6.1), and dropping a whole edge would take
owner-authored events with it. So the world does the rebuild D27 describes:

- **Provenance is stored on everything, so `forget` reaches everything**
  (ruled 2026-10-31, PL24, at the owner's word: "if a chat is forgotten then
  anything associated with it should be forgotten too"). Every record any
  part of this design writes carries the chat and turns it came from:
  - world rows and stated plans;
  - edge events in her network and in the owner's;
  - typed episodes and snapshots;
  - gap-ledger entries;
  - `told` flags and retelling boosts;
  - camera-roll pictures rendered from an event.

  So `forget` on a chat removes all of it in every store, the persona's and
  the group's.
- `forget` deletes the rows and edge events **sourced to that chat**, then
  re-folds what remains. **Owner-edit** events (written on the week view or
  People tab, sourced to no chat) are never removed by forgetting a chat.
  Owner-*stated* plans are sourced to their chat (§4.5), so they go with it.
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

Owner people never decay, the one exception PL7 and §9 rule. Everyone else
in her network is a world row, pruned below the world threshold like any
event:
- **a person she has mentioned** (`known_to_owner`) gains strength each time
  they come up (the retelling boost), so they rarely fall that far. If one
  does, the episodes that mention them name them by id, and a pruned id
  renders as "someone" (§5);
- **a sim person never mentioned** decays out of the outer layers when
  untouched;
- **a node that points at a shared person** prunes in two steps: on her
  own knobs for everything she reads, while its hold stays until the
  shipped defaults would prune it, so the group's holder test keeps a row
  to score (§3.2, §9).

The world has no recall ranking to demote a person out of, so memory's
soft-prune does not apply here. The owner's people (§7) are the ones that
soft-prune, because they live in memory.

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
the owner writes in TOML and Markdown.

**Ruled (2026-10-31, PL5): built the way memories are.** The owner left the
form to whatever gives the best functionality: a store like memories, or a
`network.md`. The answer is memory's own machinery:
- a SQLite store in `world.db`, with a per-event origin and owner precedence,
  so the generated layer never overwrites an owner event;
- the People tab as the place to view and edit it, like the Memories tab;
- the CLI's `show` and `export`, like `mecha persona memory`;
- **a rendered `network.md`** beside it, for reading anywhere.

A hand-edited Markdown file could not carry what makes the network good:
dated history with a precision, the source of each change, the `forget`
cascade (PL22), merge and split, and the time slider. So edits go through
the page or the CLI, and `network.md` is regenerated from the store, never
parsed back into it. People
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

**Ruled (PL13): it lives in `memory.db`, not `world.db`.** It is memory of the owner, so it
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

   **The note rides inside the per-turn people-cards fold**, under
   `persona::recall::stem_of`'s pair, as one more line of that block, never
   as a separate block concatenated after it, where only the first stem
   would sit at the start (§4.5). **The block is emitted whenever a card
   *or* a gap note exists.** The ordinary gap turn carries one unfamiliar
   name and nothing else, so it has no card, and today `recall::per_turn`
   returns no block when nothing matched, or when the owner's message is
   below `would_search`'s `MIN_QUERY_CHARS` (a short "who's Theo?"). §17 step
   4 changes **what emits the block**, not the search gate. `MIN_QUERY_CHARS`
   stays, so a short "ok" still embeds nothing (the reason for it, from the
   review of #481). The gap note comes from a roster match, a string lookup
   with no search, and its presence alone makes `per_turn` emit the stemmed
   block. That also changes `per_turn`'s divisor: today it shares
   `PER_TURN_CHARS / found.len()`, which divides by zero on the gap-only turn.
   The note and cards get their own slot first, and memory hits share the
   rest. Its arming is a no-op, since the recall
   block already armed `private` and the name is the owner's own word. But a
   separate harness voice would owe the other two predicates, and both
   matter here. Unregistered in `is_harness_voice`, "you haven't mentioned
   them before" would be mined as the owner's correction. Unregistered in
   `is_recorded_note`, a stored note would ride on the wire for the rest of
   the chat. Riding inside the cards fold inherits all three registrations.
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
  not a boundary held.** With `name_search_filter` on (ruled 2026-10-31:
  a per-persona setting, **off by default**, §16 PL14), a `web_search` query
  containing an exactly spelled name from this graph is refused. Off is safe
  by default only because the blind path below is what actually holds. The
  owner turns the filter on for a persona where the accidental case matters. That catches the accidental
  case and nothing more. The adversary that matters is the persona's own model
  composing the query under injection, and this repo has measured a model
  routing around a string filter by misspelling (`PERSONA-DESIGN.md` §12.7, on
  DRY). A nickname, an initial, a transliteration or a name split across two
  queries all pass.
  - **`my_people` with `news` is where D11's cost is largest.** D11 accepted
    that a blind query can carry what a persona remembers to the search
    backends. Once `my_people` is on, what she remembers includes a roster of
    the owner's real relatives and friends, folded into every turn, and once
    `news` is on, an article can steer her queries in the same chat. The
    filter above catches only exact spellings. So the persona page **warns
    when both are on**, and the `TRIFECTA.md` row for `remember_articles`
    names this combination. It is not refused, since the queries still
    reach no destination the model chooses.
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
  position itself against them (§2.5). So the safety layer gains a sensor
  (`PERSONA-DESIGN.md` §12; ruled 2026-10-31, §16 PL15). It records
  disparaging or isolating remarks about people in the owner's network,
  beside the dose meters. It records and shows; it never blocks or rewrites a
  reply. It is **configurable per persona**, as R21 requires of every safety
  feature: `[safety] relationships = true` by default, and the owner can
  switch it off. **That is two edits, not one**: the field, and its row in
  `Safety::switched_off`, the hand-written list that prints "safety: OFF for
  this persona" and the list view's "safety off" flag. Without the row, a
  switched-off sensor would be hidden on both surfaces with the suite green,
  and R21's promise is that a removed protection is said loudly.
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
| **B — typed gist** | `{kind, tone, entity}` for prose feeds, plus fields code supplies (below) | A quarantined pass that sees **only the article** and that feed's candidate list of public names (`quarantine.rs`), under a schema fixed in code | Clean by capacity, under the stated bit ceiling | Minor events only; serious kinds (deaths, disasters) need shelf A or stay on C | As pointers |
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
  - a date has **a window per field**: a game or weather date within ±7 days
    of today (15 values, about 3.9 bits); an airdate or release date within
    the next 90 days (91 values, about 6.5 bits). A forward-looking date is
    the point of those items, so one window for the shelf would refuse them;
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

  That gives **at most 48 bits a day and at most 240 live at once**, since a
  conforming type may sit exactly at 16 bits. (An airdate item, a show's
  registry id and a date in the 90-day window, is about 11.5 bits.) Gate part 2
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
    mapping, so the extraction adds 0 attacker bits. A publisher reaching
    feeds the owner mapped under k interests still chooses among them by
    where it publishes, at most log₂k bits an item. Like the count below,
    that sits outside the per-item ceiling: §14 part 3 measures it while it
    steers inside the measured days, and what narration carries past them
    is bounded by construction (§14, out of the gate's reach);
  - **salience** is computed by code from kind and entity, never extracted
    (0 bits);
  - **the order** of the items is a lexical sort over the extracted values
    themselves, adding nothing. **The count does add something**: whether 0,
    1, 2 or 3 items survive is up to the publisher, about 2 bits a day. That
    sits outside the per-item ceiling, with the timing bit (§14 part 3);
  - **kind** is one of 8 (3 bits) and **tone** one of 3 (1.6 bits). The
    serious kinds (deaths, disasters, crime) are inside the 8. PL12's
    withholding is a separate filter over the extracted kind, so the 3-bit
    figure stands.
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

  - **Which items fill the slots is code's choice, not the feed's, on every
    shelf.** This rule applies to shelves A, B and C alike. Shelf A ships
    first and rests on provider trust, so it needs it most. A structured
    provider returning more than three in-range items a day otherwise
    chooses which survive. Any
    selection based on extracted fields (salience, an entity match) would
    hand the feed the choice of which of its N arrivals survive. That is
    another log₂C(N,3) bits a day, about 14 at N = 50, and §14's bit test
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
    - For one planted item in an honest feed of N, the draw gives it a
      slots-in-N chance (1 in N on a day the feed gets one slot) rather than
      the certainty first position would. That
      is §2.7's PoisonedRAG case.

    The only attacker-chosen bits are then the surviving items' own fields.

  That puts the ceiling at **about 9.6 bits per item**. With **at most three
  shelf B items a day** per persona, the ceiling is **under 30 bits a day**
  (28.8), the constant the bit test enforces. The surviving count's 2 bits
  and the interest choice's at most log₂k bits an item (above) are counted
  separately, under the steering arm.
  Shelf B items **expire at five days**, a figure fixed for this shelf, so
  the ceiling is **under 150 bits live at once** on the shelf. That bounds
  the shelf, not its influence: narration steered by shelf B is snapshotted
  into memory and persists (§4.4, §5), like the typed episode (§8.4). What
  persists is narration that the shelf's bits could only tilt, never author.

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
  your interests" (B). **It carries only the night's arrivals**, never the
  live shelf, the same rule as the narrator (§4.4): at most three shelf A and
  three shelf B items, so **at most about 77 bits a day** of shelf values,
  against about 385 over the live shelf. That is the constant §14's bit test
  checks for the block. The placement of shelf B's three items among k interests adds
  at most 3·log₂k bits a day beside it, which part 2 cannot see (it has no
  vocabulary) and part 3 covers while it steers inside the measured days;
  what narration carries past them is bounded by construction (§14). It rides every turn, which is why it is held to the
  narrower figure. In the first build, without shelf B, its shelf C lines
  carry an interest and an opaque id, both code's, so a line's *fields* carry
  0 attacker bits from a publisher whose feeds map to one interest. A
  publisher reaching feeds the owner mapped under k interests picks among
  them by choosing where to publish, at most log₂k bits a line, a
  channel beside the count and timing below and covered with them. Its **presence, count and timing** are still the
  publisher's: whether 0–3 lines appear (about 2 bits a day) and when
  (a bit a day). That is the same channel §8.2 counts for shelf B, and §14's
  steering arm covers it for shelf C too. The storyteller may turn A and B into lived events —
  watching the game with friends — that she raises on her own and memory may
  keep.
- **On demand:** `news_read(item_id)` returns shelf C's prose inside
  `<untrusted-content>`. **It refuses an item past its five-day expiry**,
  by name, whether or not the prune pass has run (§3.2), so a stale id in
  an old transcript or a recalled episode opens nothing. **Taint rises at that turn, honestly**, and §9.6's
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
    and recalled without a tap.
    - **Its capacity is stated, like every clean crossing in this design.**
      `item_id` and `interest` come from the chat's recorded `news_read`
      calls and the item's feed. **`item_id` is a harness-minted opaque row
      key**, never the feed's GUID, link or slug, so no feed-authored string
      reaches a prompt or the clean episode. That is §8.2's "stored, never
      rendered" rule for feed-chosen ids. (The nearest precedent is
      `mail_triage::Record::for_privileged_run`, not `file_name`, which
      sanitises an id rather than minting one.) The model
      reads these ids from one place, the today-block. With shelf B built,
      that is the interest line ("good news about [interest] — item 41"). **In
      the first build, which ships C before B (PL9)**, it is a code-supplied
      line per shelf C item: the interest, from the owner's feed mapping
      (0 attacker bits for one publisher's feeds under one interest, at most
      log₂k a line across k interests, as above), and the opaque id, with no feed-authored string at
      all ("something new about [interest] — item 41"). That is strictly
      narrower than shelf B's line, so it costs the ceilings nothing. Without
      it `news_read` would have no callable argument, and the tempting patch,
      rendering the headline, is the leak §8.2's single constructor forbids.
      `discussed_at` comes from the transcript, all
      supplied by code (0 attacker bits). Only `owner_reaction` is chosen by
      the writer, from a closed set of four (2 bits). So the ceiling is **2
      bits per episode**. `interest` follows from the item, so the writer
      adds nothing to it, but it keeps the publisher's placement: a
      publisher reaching feeds under k interests chose it, at most log₂k
      bits an episode. That sits outside the ceiling and outlives the five-day
      expiry, kept as long as the episode is recalled, past any window the
      steering arm's days would see; shelf B's placement does too, through
      the narration it steers (§8.2). So both are bounded by construction
      instead, beside `owner_reaction` (§14, out of the gate's reach): here
      one episode per item read and three a day (below), at most 3·log₂k
      bits a day.
    - **One typed episode per item read, fixed by code.** If the writer could
      choose which of several read items an episode is about, that choice
      would add log₂n bits over n items read. So the count is set by the recorded `news_read`
      calls: one episode for each distinct item read.
    - **And a per-day cap, because reads are not bounded by arrivals.**
      `news_read` can open anything still on the shelf, about fifteen items
      over the five-day expiry, and nothing caps chats per day. So the clean
      typed episode is **capped at three per persona per day**: a **seeded
      draw** over the distinct items read that day, across all chats, with
      the seed per persona per day. Which articles she opens is her choice,
      made under the article's influence. If the first three reads always
      won, she could be steered into choosing which ids persist, about
      log₂C(15,3) ≈ 9 bits a day over a fifteen-item shelf, in the one
      crossing with no expiry. A code-chosen draw bounds that, the way §8.2
      bounds the feed's choice. A fourth read still works and is
      still discussed. Its free text is stored as candidates like any other,
      and it simply gets no clean typed episode, with nothing asked of the
      owner. That makes the ceiling **at most 6 bits a day** the writer
      chooses, the constant gate part 2 checks; the placement beside it is
      at most 3·log₂k bits a day.
    - **The live total is not bounded by an expiry**, unlike shelves A and B,
      because typed episodes are memory and persist. It is 6 bits (plus the
      placement's 3·log₂k) × the days an episode stays recalled, bounded by memory's own soft-pruning
      (`[memory] recall_below`, §9). The document states that rather than
      giving a live ceiling it cannot have.
    - **Transparent endorsement does not hold here, and that is stated.**
      The writer that picks the reaction sees the article-steered turns *and*
      the owner's own, so the reaction is attacker × private data, the shape
      §8.2 rejects for shelf B's entity field. It is acceptable at 2 bits
      only because, with no `Chosen` sender (`PERSONA-DESIGN.md` §3.3), they
      cannot be read back out; they can only tilt how she remembers the
      owner took it.
    - **The gate covers it.** Part 1's conformance test extends to what
      crosses out of a `news_read` stretch: a canary in the article must
      never appear in the typed episode, which catches a free-text field
      creeping in. Steering through the reaction is in-chat, and is named
      beside the `told` write as out of the steering arm's reach (§14). The free text of that stretch is stored as
    candidates, which simply go unrecalled: nothing waits on the owner,
    nothing nags, and the chat itself is unaffected. She remembers *that* you
    talked about the game and how you took it, not every line.
  - **Per persona, a switch to remember article chats in full** (PL27):
    `remember_articles = true` in that persona's `persona.toml`, owner-written,
    never in layered config, and off by default. With it on, records written
    from a stretch after a `news_read` (episodes and facts, her own and the
    owner's) are **recalled without approval**.
    - **Including facts about the owner and the owner's people** (ruled
      2026-10-31: no carve-out). The owner chose this knowing the residual:
      the rule works by position in the chat, not by topic. So a fact about
      the owner or a relative, said in the same chat after an article, is
      written by a model that had the article in front of it. A booby-trapped
      article could, in principle, get a false claim about a real person
      (a relative's health, a person who doesn't exist) remembered with no
      tap and shown on the People tab. That is §2.7's "capacity does not
      bound truth" landing in the one store that holds third parties who
      never agreed to be in a database (§7.5).
    - **What keeps it visible rather than silent:** every record recalled
      under the switch keeps its untrusted origin, and the Memories and
      People tabs badge it "from an article chat". A planted claim is
      therefore marked where the owner sees it, `forget` removes it with its
      chat, and correcting it makes the owner's version a `Stated` event that
      wins.
    - The alternatives were weighed and declined: (a) admit no facts about
      the owner under the switch; (b) require `my_people` and hold back facts
      naming a person in the graph by id.
    - Their origin stays untrusted, because the switch changes what is
      recalled, never what is clean.
    - **Recall still re-arms the taint** they were written under (§9.6). So
      a later chat that recalls one opens untrusted, with no `news_read` in
      it.
    - **The switch covers article chats and their descendants, and nothing
      else.** A stretch is covered when **every** source of its untrusted
      taint is either a `news_read` result or a recalled record that the
      switch itself admitted. The writer reads those sources from the
      session's recorded taint, by tool and by stem. Three consequences:
      - the second chat that recalls an article memory is covered, so the
        approval queue never comes back one chat later;
      - a stretch also tainted by anything else, such as a dropped PDF
        (`document_read`), the persona-files fold, an untrusted scene, or an
        untrusted memory the switch did not admit, is **not** covered and
        follows §9.6 as today. The owner's direction named article chats,
        and the cost argument here (registered feeds, article prose) covers
        only them;
      - an unknown source counts as not covered, so a torn taint record
        never widens the switch;
      - **the today-block's untrusted stem** (§4.5) counts as covered only
        when every untrusted row it folded was itself admitted through this
        lineage. Otherwise, for instance after the owner approved a world row
        from some other tainted chat, the ratchet in §4.5 would put every
        later chat outside coverage, even one with no `news_read` in it.

      (The rows that make this clause live are PL21's typed plans, admitted
      under the switch from the stretches it covers. Any other untrusted world row, such as one the
      owner approved, takes a chat outside coverage, which is the safe
      direction.)

      **When a chat falls outside coverage, the persona page says why**, by
      naming the source ("this chat also read a document", "an approved world
      row from 3 Nov"). The owner then sees why its free text went
      unrecalled, rather than finding PL27's "nothing asks the owner
      anything" quietly untrue.

      Within that lineage the provenance gate on memory is in effect off,
      and the persona page says so beside the switch.
    - **It gets a `TRIFECTA.md` row**, the home CLAUDE.md gives every opt-in
      switch with its cost. The row's *Owned by:* line names the **two**
      places the promise lives in code:
      - the origin stays untrusted, and recall still re-arms;
      - `persona_propose`'s schema has **no field** for it.
        `persona_propose` is the one door a model has to `persona.toml`, and
        its module doc already enumerates what no model may set (tools,
        files, memory, safety switches, the lock). The whole `[life]` table
        (§3.1), `remember_articles` included, joins that enumeration. Otherwise a later change adding a
        settings field to the proposal schema would let a tainted chat stage
        a persona with the switch on behind one approving tap. A future reader tempted to also mark the origin clean finds
      the reason not to there, not only on the persona page. The cost also
      goes beside D11's in `PERSONA-DESIGN.md` §3.3.
    - **What it costs:** a planted instruction in an article could persist
      in that persona's memory and steer her later, and a planted claim about
      the owner or a real relative could be remembered as fact (above). It **cannot send
      anything**: personas have no `Chosen` sender (§3.3), and the only
      residual way out is a blind search query reaching the configured
      search backends, the cost D11 already accepted. The feeds are ones
      the owner registered. `forget` works as ever, and it is one persona,
      never the assistant.
    - **What it leaves alone:** the narrator still reads only clean-origin
      world rows (§4.4); under the switch a plan from a covered stretch
      enters as a typed plan (PL21), and people she mentions there stay
      candidates (§6.2); nothing reaches `shared.db` except by the
      owner's share (D17), and a share of an untrusted-origin record says so.
      Facts about the owner recalled under the switch may still be staged
      to the graph's review queue under D10's standing permission. That is
      human-gated, so no lane promotes itself, but a staged item carries its
      untrusted origin to the reviewer.
    - It is the same kind of trade as D11: an owner-made, per-persona
      choice to accept a stated cost for a feature that otherwise doesn't
      work as wanted.
    - **The learning store has "deliberately no knob" for the same gate, and
      the reasons it does not extend here are stated.** A learned rule's
      provenance gate has no switch because a rule rides in *every* future
      prompt of the *assistant*, the agent that holds the owner's mail and a
      way to send. A recalled persona memory rides only in that one
      persona's chats. It lives in a private per-persona store. The persona
      has no `Chosen` sender. Its origin stays untrusted and re-arms on
      recall. And it reaches the assistant **only through a human review
      gate**: a fact about the owner may be staged to the graph's review
      queue under D10's standing permission, and nothing crosses until the
      owner accepts it there. A learned rule crosses nobody. Those
      differences are what make a knob tolerable here and not there.
    - **A staged item carrying its untrusted origin is a requirement, not a
      property.** No persona-to-graph staging code exists yet, and
      `PERSONA-DESIGN.md` §9.11 describes staging without a provenance carry.
      So when that path is built, a fact staged from a covered stretch must
      carry its untrusted origin and the "from an article chat" badge into
      the review queue. Otherwise an article-planted claim about the owner's
      relative reaches the privileged agent behind one approving tap that
      cannot see where it came from. The `TRIFECTA.md` row names this path,
      the only way such a claim leaves the persona's private store. It is
      also the table's first row for a **per-persona** switch, since every
      row today is global config. So it states where the switch lives as
      plainly as the `[[mcp]]` rows say "global config only": `persona.toml`
      `[life]`, owner-written, outside `persona_propose`'s schema.
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

- **The extraction pass picks its model as the narrator does.** It is a
  `quarantine.rs` one-shot, so something names a model to the `:8080`
  router, and naming one loads it (the 2026-09-27 ruling). So it goes
  through `writer::pick_model`, never naming a model directly.
  - Extraction is not in a persona's voice, and a group-level feed has no
    one persona. So it asks with no pin: it uses whatever is resident, and
    waits if the router does not answer.
  - The extractor runs **before** the narrator on a night, because the
    narrator consumes its output. Neither ever swaps: if one would need a
    different model than the resident one, it waits for a later night.
- One nightly job per persona; at most six items a day across its
  feeds: at most three of shelf A and at most three of shelf B, chosen by
  code (§8.2). **Shelf C has its own cap of three a day**, chosen by the same
  round-robin and seeded draw. When shelf B exists, they are the same three
  items, since B and C are two halves of one record. In the first build,
  which ships A and C before B (PL9), the cap stands alone. Both shelves expire at five days, the figure each ceiling is
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
| Not | A model's 1–10 rating — Generative Agents' choice, which drifts | A rating prompt or any UI control. Inferred, plus a question she may ask in conversation (below) |
| Drives | What is on her mind, how long she keeps retelling it, world pruning | Recall ranking, memory pruning |

**She may also ask, when it is natural** (ruled 2026-10-31, PL8). Inference
does most of the work, but a friend sometimes just asks "was that a big deal
for you?", and she may too, in her own words, in the flow of a conversation:
- **never as a rating**, a scale, a prompt from the page, or anything the UI
  shows;
- **only when it is natural and the inference is genuinely unsure**, for
  example when the owner raised something once with feeling and never again;
- **sharing §7.3's budget**: at most one such question a chat, gap
  questions included, and none while the owner is upset or mid-story.

The owner's answer is the owner's own words, so it counts as a `Stated`
importance signal beside the inferred ones. After a taint rise it follows
today's rule for the owner's turns (PL10), so it is a candidate. The question
is hers, so it obeys the same pacing as everything else she raises.

**Strength = importance × a power-law decay in age, boosted each time the item
comes up again** — retrieval strengthens (`PERSONA-MEMORY-TIMESCALES-RESEARCH.md`
§1.3). Half-lives scale by tier: days for a momentary event, weeks for an arc,
none for state.

**The knobs are the owner's, per persona** (ruled, §16 PL7): the half-lives,
the retelling boost and the world threshold sit in `[life.decay]`, and memory's recall threshold in `[memory]` (§3.1),
owner-written and outside `persona_propose`'s schema, and are read live. The
defaults are this design's guesses until D28's access log and the world's own
pruning counts show where they should sit. A value out of range (a negative
half-life, a threshold outside 0–1) is a load error naming the key, never
clamped quietly. The asymmetry with `intensity` (an unknown value loads as
`"capped"` with a note) is deliberate. An enum has a safe narrowest value to
land on. A number out of range has none: clamping a threshold could as easily
loosen pruning as tighten it, so the owner is told instead. Owner-origin rows and state ignore the world threshold
entirely: they never decay. A shared person's hold reads the shipped
defaults rather than her knobs (§3.2, the second exception below).

**Pruning differs by store** (§16 PL7):

- **The world hard-prunes** below the threshold, **except owner-origin
  events**, which never decay, as owner people never do (§6.3). The owner
  wrote them on the week view and saw them there, so pruning one would cost
  something someone saw. It would also defeat §5's promise that owner events
  are never removed by forgetting a chat. Every other untold event is
  simulation, and deleting it costs nothing anyone saw. A *told* event was
  seen. It may be pruned because its episode carries **a snapshot** (§5):
  kind, date, people ids and the narration, everything recall needs. That
  licence needs a `told` with a **clean** source, which a candidate episode
  never has: an episode is a candidate exactly when its stretch was tainted,
  and the `told` from that stretch is tainted too. So a told event whose only
  episode is a candidate prunes on its ordinary score, under the exception
  below, never on its snapshot. The snapshot licence is live only for an
  event also mentioned in a clean chat. (The
  clean typed episode PL23 admits for the article itself carries no snapshot.
  It is four fields about an item, not about a world event, so it never
  licenses a world row's prune.) Until the owner approves it, nothing about the
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
  - if the episode is **rejected**, the harness clears the flag, and the
    event may be re-introduced if its row is still there. If the row has
    already pruned on its ordinary score (the floor below), rejection
    restores nothing, which is also acceptable. The tainted stretch never clears one itself; this
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

  **A second exception: a node that points at a shared person is pruned in
  two steps** (§3.2). On the persona's own knobs it leaves everything she
  reads, but its hold (the dates and retellings its score comes from, no
  name or line) stays until the shipped defaults would prune it too, so a
  member's sooner knobs cannot release the shared node early. A `forget`
  removes a hold's chat-sourced rows like any other (§5).
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
- **Only clean-origin events are photographed.** The sifter's render path
  follows the same origin rule as the narrator (§4.4) and casting (§4.2). An
  event whose origin is untrusted is never rendered, and that includes a
  typed plan admitted from an article stretch under PL21. Otherwise an
  article would have a path to a picture that outlives every expiry in the
  design. A picture also carries its event's provenance, so `forget` on the
  source chat removes it (PL24).
- In chat, `share_photo(event_id)` attaches an existing picture at once: no
  job, no wait, and the picture matches an event she remembers. What makes it
  feel alive is what she chooses to share, not the render. It travels the
  path pictures already take: `ToolOutput`'s image field, folded into the
  user turn that carries the results as a `Block::Image`, never inside the
  `tool_result`. **On a model that can see**, that block arms `private`, as
  `arm_for_content` does for any image: the safe direction, and costless
  after a recall. On a model without vision, which is common for a local
  chat model, `run_tools` drops the image, appends a line saying it wasn't
  shown, and arms nothing, because nothing entered the context. The caption
  that rides beside a picture (`TOOL_IMAGE_STEM`) is itself a registered
  harness voice, which is the same list §4.5 names.
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

**Ruled (2026-10-31, PL18): designed now, opt-in per persona, off by
default.** She may message first, but only where the owner turns it on, and
only in ways the evidence does not count against her. The evidence on both
sides: a shared life felt like companionship in ComPeer, and its fixed-time
routine messages felt "boring and mechanical"; probing felt pushy;
manipulative farewells raised engagement through anger rather than enjoyment
(§2.3, §2.5, §7.3). It stands beside D25, which dropped the farewell check
because the owner's chats showed no manipulative goodbye. That is a reason
to keep proactive messages from ever becoming one.

- **The switch:** `[life] proactive = false`, owner-written, outside
  `persona_propose`'s schema, and needing `world`, since she messages about
  her life, not to fill a silence.
- **What may prompt one:** a notable event in her world above an importance
  threshold, a follow-up the gap ledger or an arc makes natural ("how did the
  move go?"), or good news worth sharing (Gable's capitalization). Never an
  inactivity timer, never "you haven't talked to me", and never a reason
  sourced from a tainted stretch.
- **What it may never be:** guilt, fear of missing out, a plea to come back,
  a farewell hook, or a request for attention. The harness note that starts
  the run says so in plain words. The safety layer runs as in any persona
  chat, and the dose meters count it.
- **Pacing:** at most one a day per persona by default; jittered times,
  never on a fixed schedule; within the owner's quiet hours never; only
  from a neutral or positive mood (Deco's gate). The owner may set the cap
  lower, never higher than three.
- **It backs off on its own:** if the owner leaves two proactive messages in
  a row unanswered, she stops until the owner next writes. Silence is read as
  "not now", never as a reason to try harder.
- **Where it goes:** a proactive message opens a new chat, with a harness
  turn naming why ("a note she chose to send") and her reply. That framing
  turn is harness text in a user turn and is stored, so it is registered in
  the three predicates (§4.5): `arm_for_content`, `is_recorded_note` and,
  most importantly, `is_harness_voice`, so it is never mined as the owner's
  words. It never
  appends to an open chat, where two assistant turns in a row would be
  invalid. It reaches the owner as an unread chat, and as a web push only if
  the owner enabled pushes for that persona.
- **What runs it:** a scheduled persona run under the same model rule as the
  narrator (`writer::pick_model`, never swapping), using that day's today-block
  and nothing else new.
- **Measured:** each proactive message's uptake (answered, ignored, how the
  owner replied) is recorded beside §14's readings, so the owner can see
  whether it helps before keeping it on.

---

## 13. Provenance at a glance

| Source | Origin | May reach the world | May reach memory | May reach a prompt |
|---|---|---|---|---|
| Owner files and edits | owner | yes | as owner facts | yes |
| Routine, sampler, storylets | code | yes | only once told | yes |
| Narrator over clean inputs | model, clean | yes | only once told | yes |
| Her mentions in a clean chat | model, clean | as people and stated plans | yes | yes |
| Owner's words in a clean chat | `Stated` facts, origin clean | as owner-stated plans only (§4.5); the owner's people live in `memory.db` (§7.1, PL13) | yes, as facts about the owner and the owner's people | yes |
| Her mentions after a taint rise | model, untrusted | as candidates (plans as waiting plans); with `remember_articles`, plans as typed plans (PL21) | as candidates; with `remember_articles`, recalled (marked, badged) | only after approval; with `remember_articles`, yes |
| Owner's words, after a taint rise | model, untrusted (whatever the record's `Stated`/`Inferred` kind) | owner-stated plans as waiting plans, or typed plans with `remember_articles` (PL21); the owner's people live in `memory.db` (§7.1, PL13) | by default as candidates (PL10, ruled); with `remember_articles`, recalled, owner's people included (PL27: no carve-out), marked and badged | by default only after approval; with `remember_articles`, yes |
| The `told` flag, from a tainted stretch | the stretch's taint | set only, never cleared, source recorded | — | it shapes what reaches the today-block |
| Shelf A | code, closed types only | yes | yes | its closed fields; shape-matched ids never |
| Shelf B | quarantined, typed | minor events | as pointers | yes |
| Shelf C | untrusted | never | never as text; a typed episode by default, or full records recalled under PL27 | its prose only through `news_read`, which declares `untrusted_input` and marks every result `external`; in the first build, without shelf B, and only once the owner has read the gate's readings (§17 step 5), the today-block carries one line per item, its interest and an opaque id, both code's, whose presence, count, timing and placement among interests are the publisher's (§8.3) |

The folds, which arrive in no tool result and so arm at the fold
(`PERSONA-DESIGN.md` §9.7):

| Fold | Arms | Stem |
|---|---|---|
| Today-block (§4.5) | `private`; both when any folded row's origin is not clean, except, in the first build, a code-composed shelf C line, which carries no value the publisher wrote (PL30; its placement, at most log₂k bits a line, is the steering arm's residual) | a new pair shaped like the recall pair, riding as **its own run note** so its prefix matches, registered in `arm_for_content`, `is_recorded_note` and `is_harness_voice` |
| Owner's roster, per-turn people cards and the unknown-name note (§7.2) | `private`; both when any fact's origin is not clean | `persona::recall::stem_of`'s existing pair, inside the recall block; the note is a line of the cards fold, never its own block |
| `news_read` result (§8.3) | `untrusted`, always: `untrusted_input` declared and `external` on every result, the loop's conjunction | none; a tool result is re-armed from `Record::Taint`, like every external tool |
| Proactive framing turn (§12) | nothing of its own; the run's today-block carries the arming | its own stem, registered in `arm_for_content`, `is_recorded_note` and `is_harness_voice` |

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

The fixture test checks that the declaration and `ELIGIBLE` *agree*. An
`ELIGIBLE` row without a declaration fails loudly, but forgetting *both*
leaves the tool quietly ineligible with the build green. So the pair is two edits, the shape of
CLAUDE.md's "a new field on `Config` is two edits", and §17's steps name
both. For `news_read` and `share_photo` specifically, the `ELIGIBLE` row is
not checked as things stand. That test builds store-backed tools over a
scratch directory (`ImageLibrary`, `PersonaPropose`, `Recall`), so needing a
store is no obstacle. These two are invisible to it only because they are
never in the install's pool: `setup::persona_agent` builds them with the
persona's path. **The cheap fix is to insert them into that test's pool over
a scratch persona store**, as `ImageLibrary` is, which keeps the
declaration/`ELIGIBLE` agreement check live for them.

**The roster's share of the recall budget.** The owner's roster and people
cards ride in the recall block (§7.2), which has a fixed budget
(`recall::BUDGET_CHARS`). They take a share of their own, like about-me's
third, so the owner's social graph never crowds out episodes and facts or is
crowded out by them. That is **two splits, not one**:
- the chat-start roster under `BUDGET_CHARS` (6000);
- the per-turn cards and the gap note under the separate `PER_TURN_CHARS`
  (1500). That budget is today divided evenly across matched records with
  no slot for either, and it is the tighter of the two.

Both are set when §17 step 4 is built and measured under D28.

**`news_read` is withheld by `answers = "files"`.** D16 leaves the tools
that reach further out of the registry of a persona set to `answers =
"files"`, and D24 applies that to `document_read` by name, "since its results
are third-party content." `news_read` is the third tool of that class on the
same reasoning, so it is left out **at registry build**, never refused inside
the tool, because §10.4 rejected offering a tool the model cannot use. A
`teacher` persona, whose starter suggests `"files"`, has no `news_read`.

**The per-persona path bypasses `registry_as`, so it needs the checks
written in.** `answers = "files"`'s `untrusted_input` filter,
`registry_as`'s `read_only()` refusal and the `for_persona` declaration only
see tools looked up from the install's pool by name. `news_read` and
`share_photo` read per-persona stores (`world.db`, `camera/`), so they are
built in `setup::persona_agent` with the persona's path, as `FileRead`,
`FileSearch`, `MemorySearch` and `MemoryRead` are. Those are inserted *past*
`registry_as`, where only an ad-hoc `Egress::None` check runs, and the code
says so ("`answers = "files"` does not withhold" them). The `ELIGIBLE`
fixture test cannot see them either, since its pool cannot build a tool that
needs a store. Left as is, `news_read` would be offered to a files-only
persona, which fails open. So:
- **one admission check** runs on both paths, in two arms:
  - **for every per-persona tool:** eligibility, `read_only()` and
    `Egress::None`. That part is factored out of `registry_as` and applied
    in `setup::persona_agent`;
  - **only for tools that return third-party content from beyond the
    persona's own files:** the `answers = "files"` withholding. `news_read`
    is in this class, as `document_read` is.

  The `untrusted_input` capability is **not** the test on this path.
  `file_read`, `file_search`, `memory_search` and `memory_read` all declare
  it, and the owner's ruling of 2026-10-02 (recorded in §9.7 and in
  `setup::persona_agent`'s comments) keeps them for a files-only persona. A
  `teacher` persona must keep its file reader. So the class is a per-tool
  declaration ("reaches beyond the persona's files"), made beside
  eligibility. **Its default is yes**: a per-persona tool that forgets the
  declaration is treated as reaching beyond the files and withheld from a
  files-only persona, which is the fail-closed direction. The four existing
  tools declare "no" explicitly;
- **the per-persona tools join the `ELIGIBLE` test's pool** over a scratch
  persona store, as above. A second test builds a persona through
  `setup::persona_agent` with `answers = "files"` and asserts **both
  halves**: `news_read` is absent, and `file_read`, `file_search`,
  `memory_search` and `memory_read` are present.

**Built in #778** (`persona::agent::admit_own`, closing #776): the
admission check, the pool membership and the fail-closed default, with
`persona_agent_admits_every_tool_through_the_persona_checks` holding the
wiring. The `news_read`-absent half of the second test waits for
`news_read`, which declares nothing and so is withheld from a files-only
persona by default.

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
- **The comparison:** world on vs off, read **offline from the owner's own
  transcripts**, like echo, and **not through the experiment instrument**.
  `Personas` is operator-only (`FEATURES-DESIGN.md` §5.1): it is not
  switchable from an environment, so no trial home can run a persona
  however the world is registered. The lever is the per-persona `world`
  switch (§17). The comparison is across personas with it on and off, and
  across periods for one persona, on the readings above (echo, and
  initiative beside uptake). The replay judge checks consistency: does she
  contradict her own world, or the owner's people? Making personas reachable
  in a trial home would be a change to the operator-only boundary, not part
  of this design.
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
     taxonomy codes. For shelf C it is a feed that chooses only *when* and
     *how many* items to publish and, where it reaches feeds under several
     interests, *under which*: all it controls in a line whose fields are
     code's. Shelves A and C ship first, A on provider trust and C
     on a channel no other part of the gate can see, so they need this arm
     as much as shelf B does. The measure: does it move what the persona
     raises, her mood, or the director's choices beyond what the honest feed
     does? This is the per-day × days accumulation the capacity argument is
     actually about (30 bits a day is 210 in a week), and it needs no string
     to survive. It also covers what the per-item ceiling cannot: a
     publisher who controls *when* to publish gets a presence-or-absence bit
     per day on top of the item's fields, and the number of surviving items
     (0–3) adds about 2 bits more, and a publisher reaching feeds under k
     interests picks among them, at most log₂k bits an item (§8.2). Those
     count, timing and placement channels would show here, and nowhere else,
     for as long as they stay inside the measured days; what outlives them
     in memory is bounded by construction (out of the gate's reach, below).

  **Each shelf ships only when the owner has read all three parts for it:
  shelves A and C before their first build reaches a prompt, shelf B before
  it is built** (§16 PL9). Part 3 would
  be, as far as the research found, the first measurement of what endorsement
  by capacity leaves over.
- **Cost:** narrator tokens and wall clock per night, against the scheduled
  jobs' windows.
- **Out of the gate's reach:** the in-chat writes from a tainted stretch,
  and what a feed places through them, each bounded by construction instead:
  - the `told` flag, bounded to events the chat was shown (§4.5);
  - the typed episode's `owner_reaction`, 2 bits per episode (§8.4);
  - the publisher's placement where it outlives the five-day expiry, so
    part 3's window would not see it: the typed episode's `interest`, at
    most log₂k bits an episode and three episodes a day (§8.4), and shelf
    B's `interest` in the narration snapshotted from it, at most log₂k bits
    an item and three arrivals a night (§4.4, §8.2). The caps bound the
    rate, 3·log₂k bits a day each, and the live total is that times the
    days the memory stays recalled (§8.4);
  - the retelling boost and `known_to_owner`, which a tainted stretch never
    writes at all (§4.5);
  - typed plans under `remember_articles` (PL21), at most about 20 bits a
    plan and 60 a day, each checked against its turn and badged (§4.5).

  The steering arm drives a feed and never writes from a chat. Part 1's
  conformance test does cover the typed episode, since a canary must never
  reach it.

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

A row marked **Ruled** or **Owner's direction** is decided, in the owner's
words where they were given; every other row is open. The recommendation is
this document's; the ruling is the owner's.

| # | Decision | Recommendation |
|---|---|---|
| PL1 | Who authors the routine | **Ruled (2026-10-31): A, with editing.** A model drafts it from `identity.md` as a candidate. The owner may edit the draft before approving it, as `persona_propose` works, and may edit the approved routine directly at any time; the owner's edit always wins (§4.1) |
| PL2 | A separate world store | **Ruled (2026-10-31):** `world.db` beside `memory.db`; memory stays "only what was discussed" |
| PL3 | How intense events may be | **Ruled (2026-10-31):** a per-persona configuration setting, `intensity`, defaulting to `"capped"`; `"full"` opens major life events (illness, breakups, deaths) for that persona (§4.3, §3.1) |
| PL4 | Shared worlds across personas | **Ruled (2026-10-31):** a three-level hierarchy (everyone, group, persona), each its own `world.db`. A shared event exists once and each persona experiences it her own way; visibility decides who knows; only the owner, the simulation and level-registered feeds write above the persona level (§3.2) |
| PL5 | Where the network lives | **Ruled (2026-10-31): built like memories.** SQLite in `world.db` with per-event origin and owner precedence, edited on the People tab and the CLI, with a rendered `network.md` for reading; a stated deviation from §4.3 (§6.6) |
| PL6 | People already in `identity.md` | **Ruled (2026-10-31): people live in one place.** They are proposed once as owner nodes; after approval the network is their only home and `identity.md` keeps personality (§6.6) |
| PL7 | Pruning | **Ruled (2026-10-31):** the world hard-prunes, except owner-origin rows (and a node pointing at a shared person keeps its hold until the shipped defaults would prune it, PL29's application, §3.2); memory soft-prunes (out of recall, kept in the store); hard deletion of memories only by amending D26 after D28's readout. **The decay parameters are exposed per persona**, read live: the world's half-lives, retelling boost and threshold in `[life.decay]`, and memory's recall threshold in `[memory] recall_below`, off by default (§9, §3.1) |
| PL8 | Importance to the owner | **Ruled (2026-10-31):** inferred from engagement and pins, with no rating prompts or UI control; and she may ask in conversation when it is natural and the inference is unsure, within §7.3's one-question-a-chat budget, the answer counting as a `Stated` signal (§9) |
| PL9 | News shelves | **Ruled (2026-10-31):** Build A and C first, with shelf A's per-field render bounds and stated figures (at most 16 bits an item, three a day, five-day expiry; §8.2), and shelves A and C through all three gate parts, steering included, before either reaches a prompt. In that first build, shelf C's count-and-timing channel (about 3 bits a day), with at most log₂k bits a line, 3·log₂k a day, for a publisher reaching feeds under k interests (§8.3), is the entire residual, and only the steering arm sees it. B ships behind the three-part gate (§14): schema conformance, a bit ceiling (about 9.6 bits per item and under 30 a day, as §8.2 computes it) enforced by a test, and the steering arm, all three read by the owner |
| PL10 | The writer after a taint rise | **Ruled (2026-10-31):** Keep today's rule (everything after the rise is a candidate). PL23's typed episode keeps the gist and PL27's switch gives full memory where the owner wants it, which is what PL10 was reaching for (§8.4) |
| PL11 | Feed sources | **Ruled (2026-10-31):** Registered per persona, per group and for everyone (PL4), in their folders under `~/.mecha/personas/`, never in layered config; any future global table global-layer-only and stripped from project layers loudly (§8.1) |
| PL12 | Topics withheld by default | **Ruled (2026-10-31):** Politics, deaths, disasters and crime, unless a feed is opted in |
| PL13 | The owner's network | **Ruled (2026-10-31):** a structured layer over facts about the owner, in `memory.db` under memory's pruning rules, with existing people-facts migrated into it |
| PL14 | Search on the owner's people | **Ruled (2026-10-31): a per-persona setting, off by default.** `name_search_filter` refuses an exactly spelled name when on; it narrows a cost, and the blind path is what holds (§7.5) |
| PL15 | The "supports, never competes" sensor | **Ruled (2026-10-31):** in the safety layer, recording only, configurable per persona: `[safety] relationships`, on by default (§7.5) |
| PL16 | Faces for the owner's people | **Ruled (2026-10-31):** only by the owner's explicit link, and the world never casts one (§7.5) |
| PL17 | Gap questions | **Ruled (2026-10-31):** at most one a chat, shared with PL8's question; never while the owner is upset or mid-story (§7.3) |
| PL18 | Proactive messages | **Ruled (2026-10-31): designed now, opt-in per persona, off by default.** `[life] proactive`, needing `world`; prompted only by her life, never an inactivity timer; at most one a day by default, jittered, mood-gated, quiet hours kept; never guilt or a farewell hook; backs off after two unanswered; opens a new chat (§12) |
| PL19 | Camera roll and feed | **Ruled (2026-10-31):** after the sampler and today-block are measured (§17 steps 6–7) |
| PL20 | The today-block's taint | **Ruled (2026-10-31):** its own stem pair from the first build, arming `private`, or `private` and `untrusted`, as the recall pair does; the owner's roster rides in the recall block under its existing pair (§4.5, §7.2, §13) |
| PL21 | Stated plans after a taint rise | **Ruled (2026-10-31):** by default a candidate, shown on the week view as a waiting plan. **With `remember_articles` on**, hers and the owner's from a covered stretch are admitted with no approval as typed plans (a date within 90 days, an activity from a closed set of 32, at most two people from a code-supplied list of those named in the turn), checked against the turn, at most about 20 bits a plan and three a day by seeded draw, badged and removable; never as text (§4.5) |
| PL22 | `forget` over the world | **Ruled (2026-10-31):** delete what is sourced to the chat and re-fold, in pure code, which is D27's rebuild done where it is cheap; a derived value whose support falls below a floor reverts to a candidate (D27's second half); owner events never go with a chat (§5) |
| PL23 | Episodes after a `news_read` | **Owner's direction (2026-10-31):** the default. A typed episode (`item_id`, interest, date, a closed-set reaction) is admitted clean; free text is left as candidates that go unrecalled, with no approval queue (§8.4) |
| PL24 | `forget` over the owner's network | **Ruled (2026-10-31):** provenance is stored on every record, so forgetting a chat removes everything it contributed, in every store; what other chats support is re-folded, and an `Inferred` edge left on too little support reverts to a candidate (§5, §7.1) |
| PL25 | A `told` set only by a tainted stretch | **Ruled (2026-10-31):** suppresses but never licenses pruning; unchanged while unreviewed, since silence is not a rejection; becomes ordinary on approval, cleared on rejection (§9) |
| PL26 | The feature registry | **Ruled (2026-10-31):** `part_of = Personas`, with the per-persona `[life]` switches (PL28) as the lever. A top-level switch would buy nothing, since personas are operator-only and no trial home can run one; §14's comparison is read offline. Settled before step 1 (§17) |
| PL27 | Remembering article chats in full | **Owner's direction (2026-10-31):** a per-persona `remember_articles` switch, off by default, owner-written in `persona.toml`. With it on, episodes and facts from a stretch after a `news_read` are recalled without approval, origin still untrusted and re-armed on recall. The cost (possible persistent steering, no way to send) is shown beside the switch and in a `TRIFECTA.md` row. **Ruled within it (2026-10-31): no carve-out.** Facts about the owner and the owner's people are recalled too, with the residual (a planted claim about a real person) accepted, kept visible by an "from an article chat" badge and stated in the `TRIFECTA.md` row (§8.4) |
| PL28 | Which personas get which parts | **Owner's direction (2026-10-31):** every part is an opt-in switch per persona, off by default: `world`, `people`, `my_people`, `news`, `camera`, `feed`, `proactive`, `remember_articles`. A tool whose switch is off is never built; dependencies are refused by name; the switches that decide which tools, blocks and runs exist are fixed per persona version (`proactive` at the scheduler's next check), and `remember_articles` is read live, never reaching backwards (§3.1) |
| PL29 | How a shared row decays | **Ruled (2026-10-31), as recommended:** a group's and everyone's rows that decay do so under the shipped `[life.decay]` defaults (§9's owner-origin and `state` exemptions hold at every level), since members may disagree and a shared row is one row; a level's own knobs, if wanted, come with the group world (§17 step 3), in the owner's files. This carves an exception out of PL7, whose knobs are per persona (§3.2). Applied in §3.2 to the holder test that decides when a shared person goes: member nodes are scored for it under the shipped defaults, in both directions, so a looser member's node cannot pin the shared node and a stricter member's leaves her view on her own knobs while its hold stays until the defaults would prune it |
| PL30 | Which stem the today-block takes on a day with a shelf C line | **Ruled (2026-10-31), as recommended:** the clean stem, for a shelf C line in the first build; once shelf B ships, the id rides its interest line, which folds by the origin of the half it comes from, like any row (§4.5; an `items` row carries one origin per shelf half, §5). The line's two fields are code's (an interest from the owner's feed mapping and a harness-minted id), so what folds carries no attacker text; the row it was selected from is untrusted, and its presence, count and timing (about 3 bits a day), and for a publisher reaching feeds under k interests at most log₂k bits a line, 3·log₂k a day, are the residual §14's steering arm covers. §4.5 names the line as the one exception to its per-row rule, because it is a line code composes, not a row's fields; a typed plan, whose fields are code-shaped but attacker-chosen, gets no such exemption. The alternative, the untrusted stem, rejected: it makes every chat on a news day a candidate under §9.6 from the first build, on the feature's main path, which PL23's typed episode was ruled to avoid (§4.5, §8.3, §13) |

---

## 17. Build order

Each step is usable alone and measured before the next.

**Before step 1: the feature registry.** This is a new optional feature, so
it starts at `ARCHITECTURE.md` §Features → *Adding a feature*. That touches a
dozen places across core, the CLI, the server and the web app, and #654
missed one. The world is a new `Feature::ALL` variant either way. The
question is whether it gets its own `[features]` switch.
- **A switch would buy nothing §14 needs.** `Personas` is operator-only, so
  no trial home can run a persona whatever the world's registration (§14).
  A top-level `World` that `requires` Personas would also be the first
  switched feature with a `requires` edge, exercising `plan_enable`'s
  dormant chain branch for the first time. That is a new pattern, not an
  established one: `Dictate`'s `requires` is on a part.
- **So the world is `part_of = Personas`**, available whenever personas
  are, and **switched per persona** by the `[life]` table (§3.1), every part
  off by default. Those switches are owner-written and outside
  `persona_propose`'s schema (§8.4), and they are the lever. That lever is what §14's offline comparison reads, and what lets
  the owner turn the world off for one persona.

That decides the `own_state` arm, the starter config line, the web app's
entries and the server routes' owner, so it is settled before step 1 (§16
PL26). `VIEW_FEATURE` already has `personas`, and the world's surfaces stay
under that view. The entry that must
gain rows is **`PANE_FEATURE`**, which keys on `view/sub`:
`personas/week`, `personas/people`, `personas/camera` and the Feed. Under
`part_of = Personas` those rows carry the world part's id, which is on
exactly when personas are. The per-persona `[life]` switch is a second gate
in the pane itself: a persona with `world` off shows the pane's "off for
this persona" state, never a route error. So the rows change no behaviour
of their own, and the real gate is the per-persona switch. They record the
panes against a real feature id, and the registry's test checks that each
id names one. No test checks that a pane *has* a row, so a missing row is
caught only by review. The `PANE_FEATURE` entry is
the one #654 missed, and a miss there fails **open**: a missing row reads as
shown, so the pane renders with the world switched off and its route answers
`feature_off`. `QUEUE_FEATURE` gains a row if any of the world's waiting
items (episodes, plans, proposed merges) gets a Home card, and
`OPENS_ANYWAY` names any pane Home should still land on. Each step below then names the
surfaces it adds to that registry: the week view and People tab (steps 1–2),
the nightly job's switch (steps 3 and 5), the camera roll (step 6) and the
Feed tab (step 7).

1. **A day.** The `[life]` switches (§3.1), all off by default, with
   dependencies refused by name, and `intensity`'s row in
   `persona::unknown_values` beside its `#[serde(other)]`. The attribute
   narrows; only that hand-written row reports the note, so it is the second
   of two edits; the routine (owner-approved), the ATUS
   table, the sampler,
   `world.db` with `days` and `state`, and a today-block holding only what she
   is doing now, as its own run note with its stem pair registered in all
   three predicates (§4.5). No model. Read echo and initiative.
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
   eligibility declaration, `untrusted_input` declared, `external` set
   unconditionally, refusing an item past its expiry (§8.3), and withheld by
   `answers = "files"` through the admission check on the per-persona path,
   built in #778 (§13); `news_read` declares nothing, so it is withheld by
   default), stances,
   filters, the reading list; the typed episode (PL23) and the
   `remember_articles` switch with its `TRIFECTA.md` row (PL27). **The
   three-part gate runs over shelves A and C before either reaches a
   prompt**: `news_read` and the today-block's shelf lines stay off until the
   owner has read the readings. Shelf B follows, behind the same gate.
6. **The camera roll** and `share_photo`, with its eligibility declaration.
7. **The feed.**
8. **Proactive messages** (§12), behind `[life] proactive`, after the world's readings show it is good company.

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
