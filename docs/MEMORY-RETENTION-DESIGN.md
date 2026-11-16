# Agent memory: what to keep, how it is related, how it comes back

2026-11-12. **Proposal for the owner's review; nothing here is built.** It
narrows and redirects the v2 design in
[MECHA-MEMORY-DESIGN.md](MECHA-MEMORY-DESIGN.md) (read §1, §7 and §31 first; this
does not restate them) and uses the field survey in
[AGENT-MEMORY-SYSTEMS-RESEARCH.md](AGENT-MEMORY-SYSTEMS-RESEARCH.md).

## 1. The goal, in the owner's words

Memory v2 is the **agent's own knowledge graph**. mecha-graph stays the owner's.
"A memory system based on appraisals and reappraisals so that the agents can
have broader knowledge and know what to store based on the interpretation and
appraisal with respect to goals, that persists beyond a chat session." Not every
chat turn in a database. Efficient, better at creative problem solving, learns
from experience, and prioritises by sensor state and charter goals alongside
the task's own goal.

```
context  = goals + constraints + measured resource state
packet   = recall(episodes, relations, concepts | context)
meaning  = appraise(packet, context)
plan     = propose_and_evaluate(meaning, evidence, constraints)
outcome  = act_and_observe(plan)
memory   = retain(outcome, sources, interpretation)
concepts = propose → evaluate → revise, within a learning budget
```

## 2. Where the build is against that

Surveyed on `feat/memory-integration-6` (the Codex branch plus six integration
slices). Three statements summarise it.

**The substrate is sound.** The append-only object and revision store with
snapshot reads, dependency closure with cascading forget, per-revision scopes,
typed `RelationInput` (predicate, subject, object, evidence span, polarity,
validity interval), `Concept` with support and counterexample excerpts, exact
quote admission, and SQLCipher key handling are general and worth keeping.

**The graph is storage without a writer or a reader.** `Memory::entity` and
`relate` have one non-test caller, a fixture. Recall is BM25 over evidence
text; `RecallQuery.neighbors` means adjacent transcript lines. A concept is
free prose (`name`, `principle`, `applicability`) with quotes, has no typed
roles, is matched on that prose and ordered by recency, and is induced by
hand: a person lists passages in a JSON file and runs `memory-session learn`.
`Situation` is echoed in the packet and never ranks; `needs` is always `None`.
`EpisodeFragment`, role binding and `memory_recompose` (the research plan's M1
and M2) do not exist.

**`retain` has no rule.** Design §7 says "at a meaningful event" and §31.3 says
"do not copy the entire host record into searchable text". The recorders that
were built capture every passage of every run and turn, seal each against the
transcript and chain each turn to the last. `appraisal::for_transcript` is
computed in the bridge and written to a private audit file; it never decides
what is kept. The six integration slices made that capture robust and attached
it to the terminal and web chats, which scaled the placeholder.

What was measured so far supports no benefit claim: outcome recall under a
changed constraint 4/8 against 4/8; the concept pilot at ceiling, 6/6 against
6/6; a learned concept delivered to none of four decisions because keyword
search never matched its prose; a local judge that did not abstain once in
four required cases.

## 3. Against the field

Most systems store everything (Generative Agents, Zep/Graphiti, A-MEM) or
extract from every turn (Mem0). Two gate writes on surprise about content
(Nemori, D-MEM). Several consolidate offline (Letta, DCPM, sleep-time compute).
Graph systems retrieve by entity, embedding or path proximity.

**Adopt, do not reinvent:** episode summaries that point at a lossless raw
record; bi-temporal facts with contradiction handling (Zep), which
`RelationInput` already resembles; prediction-error write gates; offline
induction as a scheduled, budgeted pass.

**Not found in the systems surveyed:** a write decided by appraising an
*outcome against goals and resource state*; abstractions that keep
counterexamples and are revised under a budget; recall conditioned on goals
and internal state; retrieval by role across episodes; owner, model or outside
provenance on every stored item. These are the design's claims to make good.

**The benchmarks will not show it.** LoCoMo and LongMemEval score chat recall,
and full context still beats most memory systems on LoCoMo. An outcome-graded
evaluation of our own comes first (§6).

## 4. Design

### 4.1 Four rules

1. **The transcript is the episodic record.** It is lossless, already written,
   and already carries taint. Memory copies text only for what it retains.
2. **Retention is a decision, made by appraisal, after the outcome.** Most
   runs and turns retain nothing.
3. **What is retained is related.** An episode is worth more as a node with
   typed edges than as searchable prose.
4. **Recall is conditioned on context and returns structure first.** Concepts
   that apply, then the episodes behind them.

### 4.2 The units

| Unit | What it is | Written by |
|---|---|---|
| **Episode** | One unit of work with an outcome: a run, a task, or a span of a chat that ends in an outcome. Goal and situation, what was done, what was observed, pointers and digests into the transcript, and the text of the few passages that carry it. | The host, at retention. |
| **Appraisal** | Why the episode was kept: the signed goal errors and their citations, as `appraisal` derives them. A reference to a derivation, not a second verdict store. | The host. No model call. |
| **Fragment** | A typed relation extracted from an episode: who or what, in which role, related how, supported by which span. Roles from a small closed vocabulary with an `unknown`. | One local model pass per retained episode, offline. |
| **Concept** | A relational pattern over fragments from several episodes: its roles and relations, where it applies, supporting and opposing episodes, and every evaluation it has had. | Offline, proposed by a model, admitted by the host. |
| **Interpretation** | What an episode or concept means for a stated goal. Many per episode, never overwriting. | On demand or offline. |

An episode replaces "one evidence row per transcript passage" as the thing
recall returns. Fragments are what make the store a graph.

### 4.3 Retain: the gate

An episode is a candidate when its outcome is recorded. It is kept when the
harness's own appraisal says the outcome carries information, by signals that
exist today and cost no model call:

- **Owner verdicts:** a steer, a denial, a stop, the fate of a draft, a task
  closed or reopened (`GoalError` with an owner-verdict citation).
- **Failure and repair:** `ended_on_failed_call`, failed declared checks,
  `StepFeedback` failures, a loop or no-output stop.
- **Verified success** against a declared check or criterion.
- **Surprise:** a scored prediction miss (`AppraisalStore::score`).
- **Contradiction or first support** of a concept that was recalled in the run.
- **Charter weight:** a signal on a higher-ranked charter line counts for more.

The gate is a threshold on those, with a budget: so many episodes a day,
highest first, the rest dropped. Two consequences. Owner verdicts arrive after
the run, so retention is **deferred** and runs where distill and scoring run
today (session end, the nightly pass), never inside the turn. And a reopened
task or a reversed verdict must be able to **withdraw** an episode, which the
store's dependency model already supports.

An interpretation costs a model call. It is written only for a retained
episode, by the pass that already makes one call per session
(`Distiller::appraise`), so retention adds no call to a session that keeps
nothing.

### 4.4 Relate and learn: the background pass

Budgeted, scheduled with the existing nightly rumination, local model only.

1. **Extract fragments** from newly retained episodes. Quotes are checked by
   the host; the model names roles and relations, not pointers.
2. **Propose** a concept when fragments from distinct episodes share a role
   pattern. A concept may start from one informative episode, marked as such.
3. **Evaluate** a concept when a later episode it was recalled in succeeds or
   fails, and on held-out episodes. Every result is recorded, including
   "unchanged" and "does not apply", which today are discarded.
4. **Revise:** narrow, split, merge or retire. Counterexamples are never
   dropped. A concept that keeps failing loses rank.
5. **Reappraise** on a goal or charter change and on a prediction miss, for
   the episodes that bear on it.

Turns of one conversation count as one source for support. That replaces the
per-turn dependency chain, whose cost grew with the square of a conversation,
with a rule applied where support is counted.

### 4.5 Recall

The query is built by the host from the context, not only from the model's
words: the run's goal and situation, the charter lines in play, and the
resource state.

- **Concepts first:** matched by role pattern and goal, ranked by evaluated
  usefulness, then recency.
- **Then episodes:** those that support or oppose the returned concepts, and
  those tied to the same goal or entities, through edges.
- **Then text:** hybrid keyword and vector search over retained passages, with
  a stored vector index, as the fallback. (§11.1 widens this index: over every
  transcript span, kept or not, which is another artifact with another
  privacy surface and sits under the gate rather than above it.)

Resource state conditions ranking on the host side only. The standing rule
that no sensor number, setpoint or valence enters a prompt holds.

### 4.6 Compositional reappraisal

`memory_recompose` as the research plan specified it: read-only, given a goal
and context, retrieve fragments from several episodes, propose role bindings
across them, compose a few hypotheses, check sources and preconditions, and
return them as untrusted suggestions with lineage, or abstain. It writes
nothing until an outcome tests a hypothesis. It is the first thing that needs
fragments, and the reason to build them.

### 4.7 Trust and forgetting

Every unit keeps owner, model or outside provenance, computed by the
transcript walk the integration slices built (`recording::Walk`, `Intake`).
Recalled memory stays untrusted input and arms the interlock. Concepts never
become instructions; learned rules keep their own gate. Forgetting follows
dependencies as now. Custody becomes a transcript pointer and digest checked
when an episode is retained and when it is served, in place of a seal per turn
verified on every read.

## 5. What happens to what exists

| | Keep | Change | Retire |
|---|---|---|---|
| **Store** | Revisions, dependencies, scopes, forget, relations, encryption | Add the episode unit; index concepts and fragments; persist vectors | Rebuilding the search index per query |
| **Capture** | The provenance walk; the terminal and web hook points; incognito exclusion | Hooks call the retention pass, not a recorder | Per-turn sealing, the turn chain, capture of every passage |
| **Learning** | Exact-quote admission; counterexamples kept across revisions | Typed roles; recorded evaluations; a scheduler in place of hand-written inputs | Induction from hand-picked passages |
| **Dynamic goals** | Windowed owner lines, compact decision records, replay from them | | |
| **Trial harnesses** | As regression tests for the store | | New versioned stores and protocols |

The open pull requests #846 and #862 to #871 stay unmerged until this is
decided. Their recorder work is not thrown away; it becomes the provenance and
hook layer under a different caller.

## 6. Evaluation comes first

Before any of §4 is built, the benchmark the research plan called M0: paired
cases in `mecha exp` with deterministic graders and hidden world state.
Families: goal switch on one episode; recombination across two; an analogy
that must be refused; a contradicted concept; a changed resource state; a
no-change control. Arms: no memory, raw transcript retrieval, retained
episodes, episodes plus concepts, plus recomposition. Equal evidence and equal
budget across arms. Cost of writing and learning counted, not only of
answering.

**Stop rule:** if giving the unchanged model the same source episodes matches
concepts and recomposition on held-out cases, the extra structure has not
earned its place, and the effort goes to retrieval.

## 7. Build order

1. The benchmark and its baselines (no memory, raw retrieval).
2. The episode unit and the deferred retention gate, on existing appraisal
   signals. Measure how much is kept and whether recall of episodes helps.
3. Context-conditioned recall with concepts first, over hand-seeded concepts,
   to isolate retrieval from learning.
4. Fragment extraction and typed concepts, with recorded evaluations.
5. `memory_recompose`, then the scheduler and budget.

Each step is measured on the benchmark before the next begins.

## 8. Decisions (owner, 2026-11-12) and what they settle

1. **Agent memory is the single home for interpretations.** `appraisals.jsonl`
   is the earlier system this one improves on. Its producer
   (`Distiller::appraise`) writes into agent memory, its clean-only reader
   policy carries over, and the old store is read-only history once callers
   have moved (§31.6 of the v2 design gives the retirement rule).
2. **The grain of an episode** was left to this document. An episode is a span
   of work bounded by an outcome. A run or a task is one. In a chat, the
   consolidation pass cuts the transcript at boundaries the host already
   records, with no model call: a declared check or criterion result, an owner
   verdict, a task closed or reopened, a change of goal-context revision that
   is not a steer, a compaction, and the end of the session. A span that ends
   in none of these and trips no retention signal stays in the transcript
   only.
3. **Concepts are the creative half and the least built, so they get the most
   design attention; they do not replace reflection.** The owner's working
   definition (2026-11-12): a concept is closer to semantic knowledge, and
   also a relational pattern of information that can be reused and referenced
   as a thing of its own. The construct is not yet well defined and is to be
   iterated on. Concepts are their own lane, delivered by recall as
   hypotheses, never as instructions. The reflection system and its lessons
   (reflect, learn, learned rules, goal lessons) stay as they are. Whether
   concepts could later take over any of that is a question to answer with
   measurements, only if concepts prove good enough, and is not a plan. The
   purpose the owner gave is that the system must be able to suggest what it
   does not already know, so that appraisal and episodic memory do not loop on
   what they have already seen; role binding across episodes is the mechanism
   proposed for that.
4. **Consolidation runs in downtime.** Nothing is written inside a turn. The
   pass runs when a chat is idle, at session end, and nightly, and it yields
   to a live run the way other background work does.
5. **Everything agent-owned moves into agent memory.** Distill stops writing
   `agent:mecha` episodes into mecha-graph once episodes live here. mecha-graph
   may later be redesigned or migrated into this system; that is not part of
   this design.
6. **Custody is a pointer and a digest**, left to this document with the
   constraint of low compute and storage overhead and fast search. The digest
   is checked when an episode is retained. At serving, the check is a file
   identity comparison (device, inode, size, change time) against the one
   recorded at the last full check, and a full re-hash only when that differs.
   Retained passages keep their text in the store, because search, relation
   spans and concept excerpts need it; nothing else is copied.
7. **The benchmark's tasks** were left to this document: work that is relevant
   to what the assistant does, and problems that are otherwise out of reach
   for a small local model. §6 is revised accordingly below.

### 8.1 The benchmark, revised

Two tiers, both in `mecha exp` with deterministic graders and hidden state.

**Tier A, assistant work.** The fixture mail, calendar and board world with
fictional staff. Goal switch on one episode; recombination across two; an
analogy that must be refused; a contradicted concept; a changed resource
state; a no-change control.

**Tier B, problems a small model cannot solve cold.** Tasks built so that one
run fails or exhausts its budget without prior experience, and experience is
the only route: a tool with an undocumented quirk that an earlier run
discovered; a multi-session repair where the cause was observed in one
session and the fix belongs in another; a procedure that needs two lessons
learned on different tasks combined; a long task that must survive compaction
and a restart. The measure is whether the same model, same budget, succeeds
where it did not, and what the memory cost to get there.

### 8.2 Still open

- The role vocabulary for fragments: how small it can start, and who extends
  it.
- Whether a concept recalled into a run may be evaluated by that run's
  outcome when the run also read outside content (provenance of the
  evaluation).
- What the web app shows the owner: retained episodes, concepts and their
  evidence, and a way to forget one.

## 9. Concepts: a working design to iterate on

2026-11-12. The owner's framing: there is an episodic store and situated
appraisal; what is missing is a flexible relational learning system. A concept
may be a relation configuration that a model call has labelled and that is
then retrievable by that name, or the configuration itself may be the concept,
retrieved and reused to pull out information in a new context. The construct
is to be refined by building it.

The literature this rests on was surveyed at abstract level, much of it cited
from memory; the claims below are a design basis, not verified findings.

### 9.1 Both forms, as two stages

Structure-mapping theory treats an analog as a set of relations, a mapping as
a one-to-one alignment that prefers systems joined by higher-order relations,
and an inference as the relations of the remembered case that the new case
does not yet have. People retrieve analogs mostly by surface similarity and
judge them by shared structure, and the MAC/FAC model builds retrieval as
those two stages: a cheap filter over everything, then structural matching on
the few survivors. Embedding retrieval has the same surface bias.

So a concept has both forms, with different jobs:

- **The configuration is the concept.** A small pattern of typed relations
  over role slots, with at least one higher-order link (`causes`, `enables`,
  `prevents`) between relations. Its identity is the structure.
- **The name is an index and a handle.** A model call may give the pattern a
  name and a one-line gloss. That makes it findable by keyword and embedding
  and lets other concepts and episodes refer to it. A pattern can exist and
  be used before it is named; library-learning work reports that naming
  abstractions helps reuse.
- **Reuse is filling slots.** Bind the roles a new situation satisfies; the
  relations left unbound are the output, returned as hedged hypotheses. This
  is how a concept proposes something not seen verbatim.

Label-only memories (insight lists, reasoning banks, community summaries)
retrieve well within a domain and have no mechanism for the second half.

### 9.2 First implementation

- **Extract** on consolidation of a retained episode: one local model call
  returns at most a dozen relations in a closed vocabulary of some fifteen to
  thirty predicates over typed entities, with an `other` escape that is
  logged and never matched. Open predicates fragment ("blocks" against
  "prevents") and structural matching then fails silently. Extract twice and
  keep the agreement.
- **Propose** without a model call: find earlier episodes by a vector of
  predicate counts (independent of surface wording) and by embedding, align
  greedily on identical predicates with compatible argument types, and emit a
  candidate when the overlap is at least three relations including a
  higher-order one. Comparing a success with a similar failure gives the
  condition under which the pattern holds. One model call names a candidate.
- **Match** in two stages: predicate vector and label embedding pick a
  handful; a subgraph match with slots as variables accepts or rejects; the
  unbound relations come back as predictions.
- **Evaluate** by counting: how often each relation held when the concept
  matched, and predictions confirmed over predictions made. Prefer the most
  specific pattern consistent with its positives; widen only when a positive
  requires it; a counterexample adds its difference as a condition. Split a
  concept whose matches divide on one relation. Retire one that predicts
  nothing its parts do not.
- **Store** as the relation, role and binding-log shape this implies, on the
  existing revision store, with the binding log as the evaluation history
  the current code discards.

### 9.3 What is not known

- Nobody has shown structural matching beating label retrieval on relation
  graphs extracted by a language model. This is the main bet and the first
  thing to measure.
- Extraction noise from a small local model is the dominant practical risk
  and the least studied.
- The vocabulary size is a guess.
- Transfer by candidate inference has been demonstrated on hand-coded
  representations, not on agent traces.
- Language models handle near analogies and degrade on far ones and on
  surface-similar distractors, which argues for the structural check being
  code and not a prompt.
- The idea that prediction error gates what is stored has behavioural support
  and a contested neural account; it is a design heuristic here.

### 9.4 What to measure

Prediction precision against a label-only ablation; the share of useful
matches whose label similarity was low (reminding across domains); agreement
between two extractions of one episode; concepts per episode and the fraction
ever reused.

## 10. Built so far (2026-11-13)

`tool::episodes` is the first form of §3 to §5, measured on the benchmark's
four families (the numbers are in PR #875 and are not repeated here, where
they would go stale). An episode is kept after a run from its own record with
no model call: a refused call and what fixed it, a server tool's first
working call, and the task's outcome in the run's own last words. It is
handed by the harness to a later run that has the tool. A grade given after
the run retires a failed task's conclusion.

Two things to keep in step, because only `episodes::fold` is shared between
them: the file-backed verdict is a line `episodes::load` applies when it
reads, and the store-backed verdict is a revision of the outcome record with
`judged` set (`SessionMemory::appraise_episode`).

`tool::concepts` is the first form of §9, with no model call anywhere in it.
After a run, `concepts::extract` reads the run's own calls for an arrangement
of one of two kinds (`concepts::Kind`). A bracket: the thing most tools were
called on, the change that was made to it, the last other call that worked on
it before the change, and the last one after. A linked change: one tool that
worked on two things with every other argument the same, where an earlier
call's result named both. A tool is kept as a role by the types of its arguments and never their
names (`ToolShape`), so two services that share no word can show one
structure. Runs with the same structure fold into one concept, a run judged
to have failed supports none, and a concept handed to three runs without once
being borne out is not handed to a fourth.

It comes back in two ways, and the measurements say the second is the one
that matters. At the start of a run whose tools fit the roles by shape, the
harness hands the arrangement over as a hypothesis with the candidate tools
for each role. Then the harness holds the last step open
(`agent::SuccessCue`, `concepts::Watch`): when the change works it says a
step is still owed; when the run repeats an earlier lookup and a field
differs from what that lookup first showed, other than the change that was
asked for, it names the field; and when the thing is back as it was found it
says so, because a run told only that a step is owed keeps searching after
it has made it. For a linked change it says, when the change has worked on
one thing, that the same call on a second is owed, and lists what the first
thing's record names beside itself. Everything said after the start is the harness's words and
this run's own tool names and results.

What this form does not do: it knows two kinds of arrangement and each is
code, so a third is a third extractor; it reads a thing's record only from
lookups that return a flat object; it neither names a concept nor compares
two; and it learns an arrangement only from a run that got it right. What a
failed run teaches is below.

**A grade is a reappraisal, and a failed task is kept as one.** The first
form dropped the conclusion of a task judged to have failed. That left a
lifetime of failures with nothing to learn from, and the linked pairs showed
it: where the experience tasks failed, no arrangement was kept and nothing
carried. `Episodes::reappraising` hands the failed outcome to a later run
that has the same tools as a failure: what that run did
(`Episode::did`), that it was judged to have failed, and which of the tools
here it never called. The conclusion itself is still not handed over as
knowledge. Handed over at the start alone, it was not acted on, as with
concepts; so the harness also says it at the point the later run has used
every tool the failed run used (`episodes::Retraced`, a second
`agent::SuccessCue`), which is where the earlier run stopped. Nothing here
says what was missing, because nothing recorded does: the grade is a bit.

Not built: goal- and state-based appraisal, reappraisal by anything richer
than a grade (a later run's success contradicting an earlier conclusion, an
owner's correction), fragments and the model-proposed stage of concepts
(§9.1), `memory_recompose`, and consolidation in downtime. A call episode
kept from a failed task ("it worked the first time, called with…") is still
handed over beside the failure it came from, and reads as approval of the
call that was not enough. A verdict retires a conclusion and not the call episodes kept from
the same failed task; whether it should is open.

## 11. What is recorded, what is built, what fades (2026-11-16)

Proposed on 2026-11-16 and taken up by the owner the same day ("this sounds
like a good start let's proceed with implementing the proposal"). It is a
starting shape to build and measure, not a settled one. It answers a
question §4.3 left sharper than it should have: if appraisal decides what is retained, what happens to a turn
that carried no signal when it happened and matters later, under a goal
nobody had then?

### 11.1 The gate decides what is built, never what exists

§4.1 already says the transcript is the episodic record. The consequence was
not drawn: a retention gate that decides *existence* cannot be wrong cheaply,
and one that decides *priority* can. So there are three layers, and the gate
sits between the second and the third.

1. **Recorded: everything.** The transcript, as now. Nothing here is gated,
   and nothing is copied out of it per turn.
2. **Findable: everything, weakly.** A stored index over transcript spans:
   keywords and one embedding per span, a pointer and a digest (§8 item 6),
   no model call beyond the embedder and no seal. This is what keeps a span
   that was unremarkable then findable now.
3. **Built: what an outcome marked.** Episodes, verdicts and concepts, by the
   signals of §4.3. This is the only layer the harness hands to a run or cues
   during one.

What stops is the sealed copy of every turn into the store with each turn
depending on the one before, whose cost grew with the conversation and under
which forgetting an early turn withdrew every later one.

The measurements behind the split are the benchmark's (PR #875 and after):
a store of everything that the model had to ask for was never asked;
everything delivered by keyword helped where the lesson was an observed fact,
and for more tokens, and failed where the earlier run had been
confidently wrong, because the wrong conclusion was delivered unmarked;
appraised episodes passed both for less. Storing without discrimination cost
nothing there, where what was stored was text and its keywords; the second
layer adds an embedding a span, whose cost at the size of a real history is
not measured. Delivering without discrimination did cost.

**What the second layer hands over is prose, and that is its largest new
surface.** Every other recall path returns something typed or built; this
one returns a passage as it was read, from any session of the home. Its
rules are therefore the ones for prose a run did not write: a passage is
never an instruction and never mined as the owner's words; it carries the
home's private data into the conversation that receives it, whichever
session it came from, and is marked so; what was outside content when it
was first read is outside content when it is recalled; and a session kind a
privileged run is never shown (a front-door conversation, a test) is not
indexed at all. Whether a recalled passage may be shown without a warning
around it, and whether its origin may ever be cleared, are the owner's
rulings and are recorded with the build (§11.6).

### 11.2 Promotion is retroactive

A span moves from the second layer to the third when something later makes
it matter:

- **A verdict or a failure** promotes the spans it bears on, not only the
  turn it landed in.
- **A change of goal or charter line** runs a pass over the spans tied to it
  (§4.4 step 5), which may build episodes from turns that tripped no signal
  when they happened.
- **Use.** A span found by the broad index and followed by a success is
  promoted. A downstream outcome is a label that costs nothing to compute.
  It is not free of trust: this is the first signal whose antecedent is the
  span's own content, so fetched text that nudges a run toward what a grader
  calls success would buy itself a place in later recalls. §8.2 has the same
  question open for concepts and it is not answered here. Promotion by use
  therefore classifies the span's origin as the integration's recording walk
  does, fail-closed, promotes nothing whose origin is not clean, and is the
  arm measured last rather than assumed.

So a miss by the gate is a delay and not a loss. And whether a thing is
meaningful is not answered once at writing: an episode has many
interpretations, one per goal (§4.2), written at recall or in downtime.

### 11.3 Fading and pruning

The two layers are pruned differently, because they fail differently.

**The built layer is pruned actively.** What is handed to a run is imitated,
and a superseded entry is the worst distractor there is: closest to the
query, and wrong.

- **Rank decays with disuse.** Nothing scores this yet. `replay_priority`
  (gain × need × decay) is the nearest thing and is not it: its decay is by
  age alone, its need counts recent runs in the same situation and not uses
  of the entry, and it ranks sessions for replay, not entries for recall. A
  count of when an entry was last handed over and borne out has to be kept
  per entry, and is to be built.
- **Supersession, never overwrite.** A contradicted episode is marked invalid
  from a time and kept. It stops being handed over and stays auditable.
- **Retirement by outcome**, as a concept already has: handed over some
  number of times and never followed by success, it is not handed over
  again. The number is to be measured; retiring on few trials is reported to
  hurt.
- **A cap on what may be handed over at all**, however large the store.

**The recorded layer fades and is not deleted.** A mundane span is one
nothing promoted. It has no rank of its own to decay: it is found only when
something asked is close to it, which for most spans is never, and that is
all forgetting needs to be here. It can still be promoted. Deletion is the
owner's alone, by forgetting the session (`mecha-core/src/forget.rs`), which
removes the transcript and what every store derived from it; the index of
the second layer is one more store that walk has to be taught. The
prototype store's own `forget` purges that store only and leaves the
transcript, so it makes a span unfindable there and deletes nothing.

### 11.4 What is not known, and the measurement

- Whether retrieval over the second layer is good enough. The benchmark's
  transcript arm, on keywords alone, mostly returned earlier prompts.
- Whether promotion by use works at all. Nothing here has tried it.
- The retirement and decay thresholds.

One family answers the first two: a fact that was unremarkable when it
occurred and is needed later under a different goal. Arms: no memory, which
is the control and the only thing that shows the probe is not solvable
cold; the gate alone;
the gate with the broad index; everything delivered; and the gate with the
index and pruning, on a lifetime long enough to hold a superseded fact.
Predictions are written in the manifest before the run, and the family is
held out until then.

### 11.5 What this changes in the build order

A stored index moves up beside the gate (§7 step 2, which still reads as
written before this section, as does §5's Capture row: what is retired
there is the sealed copy of every passage into the store, not the index
over them), and it is not the one
§4.5's third bullet described: that one was over retained passages, above
the gate. This one is over every transcript span, under the gate, and the
gate is only safe to trust with this one under it.

### 11.6 Built so far (2026-11-16)

The second layer, in the form that is measured first: `tool::spans` and
`agent::ResultRecall` (`mecha run --spans-deliver`, the experiment arm
`spans`). The measurements are in the pull request that added them and are
not repeated here, where they would go stale. What they changed in this
section's own claims:

- **The index is asked after a call, with what the call returned.** §11.1
  said the second layer keeps a span findable; it did not say when it is
  looked for. A task's words rarely name what it will turn out to need, and
  the result of a call does: the record a run is reading is close to that
  record's earlier reading, and a vessel's stated need is close to the
  notice that bears on it.
- **Meaning is the floor and words give the order.** The embeddings server
  scores two records of one shape as the same, so meaning cannot order
  them. With the two ranks fused evenly, a later and emptier reading of
  every record crowded out the one reading that mattered.
- **Being handed the right thing is not acting on it.** On the yard
  lifetime, once retrieval was right, the model often set the hand-over
  aside in favour of a present record that was merely silent, citing the
  hand-over's own caution that it may no longer hold and the outside-content
  wrapper around it. How memory is framed when it is handed over, and what
  the harness says when a remembered fact is absent from what is in front
  of the run, are open, and are the owner's to rule on.
- **The yard lifetime is development data.** It was run once as held out;
  its fixture and the index's ranking were then changed on what that run
  showed. It also turned out to test a conflict between memory and a
  present record, which §11.4 did not intend: the next family puts the fact
  where the later task cannot look at all.
- **A known blind spot: a number is not news.** The restatement rule counts
  the words a passage adds to the query and leaves numbers out, so that an
  acknowledgement differing only in a byte count brings nothing to mind. The
  cost is that an earlier record differing from the present one only in a
  number, a setting whose value changed, is taken for the record said again
  and is not handed over. Not measured; the superseded family is where it
  would show.

Not built: the gate beside this index in the web chat, promotion (§11.2),
and every rule of §11.3. The index is a file in the mecha home, not yet in
the encrypted store.
