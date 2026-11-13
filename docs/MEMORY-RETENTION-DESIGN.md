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
  a stored vector index, as the fallback.

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
After a run, `concepts::extract` reads the run's own calls for one
arrangement: the thing most tools were called on, the change that was made to
it, the last other call that worked on it before the change, and the last one
after. A tool is kept as a role by the types of its arguments and never their
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
it has made it. Everything said after the start is the harness's words and
this run's own tool names and results.

What this form does not do: it knows one kind of arrangement (before, change,
after, on one thing), it reads what a step left behind only from lookups that
return a flat object, and it neither names a concept nor compares two.

Not built: goal- and state-based appraisal, reappraisal from later runs,
fragments and the model-proposed stage of concepts (§9.1), `memory_recompose`,
and consolidation in downtime. A verdict retires a conclusion and not the call episodes kept from
the same failed task; whether it should is open.
