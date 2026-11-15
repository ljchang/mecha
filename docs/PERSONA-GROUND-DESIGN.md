# Persona common ground: what the two of them share, and how it is said

**Status:** proposal, 2026-11-14. Nothing here is built. §10 lists the nine decisions that are
the owner's; §9 is a build order that measures before it fits.

**The question:** a persona chat still reads as talking with a machine. It repeats what it has
already said, explains what both sides already know, brings up the wrong thing or the right
thing at the wrong moment, and never refers back to something from earlier in a new way. What
would it take for what is referenced, and how, to work the way it does between two people?

**The answer in one line:** people do not retrieve facts into a conversation; they keep a record
of what the two of them share, say less and less about it as it becomes shared, and are reminded
of it by what the other person just said. A persona today has memory of facts and no record of
what is shared, so it can only restate. This document proposes that record (the *ground*), four
behaviours built on it, and the log that makes it an instrument for studying conversation.

**Why it is also an instrument.** The owner studies how memory and context work in real
conversation, and asked that the system "track the graph dependencies and when and where they
are referenced and the timescale". Every mechanism below is a typed event with a time, so the
record of a chat can be read back as a graph of items, cues and mentions (§8). Where the
literature has no answer (§2.6), the system can run the experiment.

Privacy: this document carries no conversation text, no persona names and no session ids. Its
examples are made up or quoted from published papers.

---

## 1. What exists, and where it stops

Read from the tree at `b0995402e` and unchanged at `b3b6e2c78` (one visibility change in `persona::memory`). Symbols, not lines.

| What | Where | Limit |
|---|---|---|
| Told-ledger for three teller lines | `persona::today::Ledger`, held in `PersonaSession::told` | Word match on closed English lists; in process memory only, so a restart re-offers every line; covers events, feeling and weather and nothing else |
| Per-turn recall | `persona::recall::per_turn`, keyed by the owner's words | Whatever is retrieved is put in the prompt; the only filter is `already`, a 60-character prefix match against the conversation |
| Chat-start memory block | `recall::chat_start` | Chosen by budget and recency, not by what the chat turns out to be about |
| Episodes carry `open_threads` | `persona::memory`, written nightly by `persona::writer`; `recall::chat_start` and `Memory::recall_search` append them to an episode's line | They ride in every chat-start block and in per-turn recall as text. Nothing gives a thread a time it falls due, records whether it was raised, or closes it when the owner passes |
| A record of when a memory was used | Ruled as D28 (`PERSONA-DESIGN` §9.13), unbuilt | There is no record anywhere of a memory being recalled, offered or said |
| The typed "shown set" beside `Record::Notes` | Designed in `PERSONA-LIFE-DESIGN` §4.5, unbuilt | A transcript says what text the model was told, not which items rode in it |
| Memory v2 | `memory_session`, feature `memory-prototype` | Not wired to persona chats ("a persona chat has its own routes and its own memory"); pulled by the model through a tool, with no notion of a cue |
| Repetition readouts | `persona::echo`, `persona::initiative`, `mecha persona measures` | Offline counts of repeated wording and of who raises people; nothing about items |

So the pieces that exist answer "what does the persona know?" Nothing answers "what do the two
of them share, what has each been told, and what is live right now?"

## 2. What is known

Each claim carries how it was read: **[read]** the paper's text, **[abs]** an abstract or a
summarising reader over the full text, **[recall]** not re-read for this document. A claim
marked [abs] or [recall] is checked before anything is built on it.

### 2.1 Common ground is built, and it has two kinds

- Something is shared only once both sides believe it was understood "to a criterion sufficient
  for current purposes". Evidence of understanding comes in three forms: an acknowledgement, the
  relevant next turn, and continued attention (Clark & Brennan 1991) [read].
- Common ground is *communal* (what members of a shared community can assume) or *personal*
  (what two people experienced together); "personal common ground defines friends versus
  strangers" (Clark 1996, ch. 4) [read].
- A pair settles on its own term for a thing and keeps it: after the context no longer required
  the specific term, speakers kept it about half the time, more the more it had been used, and
  dropped it quickly with a new partner (Brennan & Clark 1996) [read].
- People are worse at remembering whom they told than who told them (Gopie & MacLeod 2009)
  [abs], and remembering shared history is not enough: speakers often fail to use it unless they
  see it as relevant (Yoon, Benjamin & Brown-Schmidt 2016) [abs].
- Being forgotten lowers felt closeness (Ray et al. 2018) [abs].

### 2.2 Shared things are said in fewer words, and the mechanism is known

- Describing the same figures to the same partner, speakers went from 41 words per figure to 8
  over six trials, and from 3.7 turns to about one. The principle: both parties "try to minimize
  collaborative effort" (Clark & Wilkes-Gibbs 1986) [read].
- Which words survive: the ones that were distinctive when the thing first came up. A word used
  for many things at first was less likely to become the label; the most distinctive initial
  word persisted 24–31% of the time against 2.5–6.6% for a random one. Function words go first.
  Reduction was greater after the listener got it right than after an error, and the effect was
  specific to that referent (Hawkins, Frank & Goodman 2020) [abs].
- Two levels: what a pair builds sits on top of a community-wide prior. With a new partner the
  wording lengthens again, though less each time (Hawkins et al. 2023) [read, selected
  passages]. The authors concede a practice effect could explain part of this, and note their
  decay model lacks consolidation and source memory.
- An outsider cannot unpack it: overhearers who heard every word identified referents less
  accurately than the addressee, 88% against 99% (Schober & Clark 1989) [read].
- **Language models do not do this unprompted.** In repeated reference, human pairs went from
  about 20 words to 6 while model pairs stayed at 58 to 56 (Jones et al. 2026) [abs]; models'
  word reuse was no higher with their real partner than with a shuffled one (Wang et al. 2026)
  [abs]; prompting alone did not make multimodal models more efficient over repetitions (Hua &
  Artzi 2024) [abs].

### 2.3 Saying less is what closeness sounds like

Templeton et al. (2024, PsyArXiv g38cx) [read, main text] call a moment whose meaning for the
pair is opaque to an outsider *insider language*, and find:

- Friends used it on more turns than strangers, and more the more often they talk.
- Friends spoke fewer words in ten minutes and needed far more words to unpack what went unsaid
  (mean 186 against 49).
- Strangers' insider language is mostly shared expertise; friends' is people and experiences,
  and spread over more categories (higher entropy over the kinds of thing referred to).
- Turn by turn, insider language tracks the speakers' own ratings of connection, more strongly
  for strangers. In a lagged model it precedes connection, peaking five to seven turns later,
  and higher connection predicts *less* insider language next.

This is temporal precedence in observed conversation, not a manipulation. Related evidence is
correlational too: private idioms track closeness in couples and friends (Hopper, Knapp & Scott
1981; Bell & Healey 1992) [abs]; couples reach for shared references when their shared reality
is threatened (Rossignac-Milon et al. 2021) [read].

### 2.4 What is live, and what brings something back

- How a thing is referred to signals how active the speaker takes it to be for the listener:
  a pronoun for what is in focus, a full description for what is not (Gundel, Hedberg &
  Zacharski 1993; Ariel 1990; Chafe 1994) [recall]. There is no agreed rate of decay.
- Recall in conversation is cued, not scheduled. Retrieval depends on overlap between the cue
  and how the thing was stored (Tulving & Thomson 1973) [recall]; most involuntary memories have
  an identifiable cue, and a cue tied to many memories brings back none of them (Berntsen 2009)
  [abs].
- People freely recall a small fraction of a conversation's ideas minutes to weeks later; gist
  stays and wording goes, except for striking lines (Brown-Schmidt & Benjamin 2018) [read].

### 2.5 Unfinished things

- A question makes its answer due, and its absence is noticed (Schegloff & Sacks 1973) [recall].
- Interrupted tasks are not remembered better, but people do tend to resume them (Ghibellini &
  Meier 2025) [abs].
- Follow-up questions predict liking within a conversation (Huang et al. 2017) [abs], a result
  that has been contested. No experiment was found on following up *across* conversations.

### 2.6 What memory does to an assistant

- Adding memory makes over-mention worse: a score of appropriate use fell by a quarter to a half
  with memory added, and the more elaborate systems fell furthest (Hu et al. 2026, OP-Bench)
  [abs].
- Recall does not buy satisfaction: over four months and 1,872 sessions, question-answering
  recall rose from 19.7% to 70.1% across memory conditions with satisfaction unchanged. The one
  behaviour linked to satisfaction was weaving in a memory the user had just cued (ρ = 0.29 over
  48 sessions). Unprompted recalls showed no benefit; mistimed ones were negative (8 cases), and
  users picked up a recalled topic 19 times in 64 (Sumida, Inoue & Kawahara 2026) [abs]. Small
  samples, observational.
- Referring back in paraphrase was received better than verbatim (Cox, Lee & Ooi 2023) [abs].
- No system surveyed keeps a record of what it has told the user, decays an item on mention, or
  carries an unfinished thread across sessions with a time it falls due. That is from a bounded
  search, not an exhaustive one.

### 2.7 What nobody has measured

- Whether a callback *causes* felt connection. Every result is correlational or lagged.
- What separates a callback that lands from one that grates: distance, how new the use is, who
  coined it.
- How often re-telling is noticed, and at what lag.
- Whether following up across conversations changes closeness, and where it starts to read as
  surveillance.
- Any of this on a small local model.

## 3. What follows for the design

1. **The unit is an item the pair shares, not a fact the persona knows.** It has a short handle
   and an unpacked meaning. The persona stores both and says the handle (§2.3's unpacking task
   is this record).
2. **The default is silence.** An item enters a reply because the owner's words cue it, not
   because retrieval returned it (§2.4, §2.6).
3. **Compression is earned.** A reference shortens only after the owner showed they followed the
   longer one, and lengthens again when they did not (§2.1, §2.2).
4. **The handle is made from the words that were distinctive at first mention**, not from a
   summary a model writes (§2.2). The writer's pass may only pick spans of wording the chat
   used; who may author a handle is §10's seventh decision.
5. **Two levels.** The persona's written character and general knowledge are the communal
   prior; the ground is personal, per persona, and nothing in it crosses to another persona
   (§2.2).
6. **Growth follows history.** Early, what can be compressed is shared interests; people and
   events come as the pair accumulates them (§2.3).
7. **Sparse.** The effect on connection lags and saturates (§2.3); one item a reply, with a
   rest after use.
8. **The model has to be told.** A model given the right fact does not use it, and does not
   compress by itself (§2.2, §2.6). The harness names the one item and the form to use.

## 4. The ground

Per persona, in `memory.db` itself: tables for **items** and an append-only **event log**. In
the same database as the rows they are made from, so that a forget erases all of it in one
transaction (§7).

An item:

| Field | Meaning |
|---|---|
| `kind` | A closed set: a person, a place, an event, a running joke, a coined term, an unfinished thread, a stated plan |
| `handle` | The pair's forms for it, longest first. Each form carries the chat it was first used in, as its own deletion key, and its own `origin` |
| `meaning` | What an outsider would need to be told. It may ride as background, so the persona understands a handle the owner used (§5.2); it is never given as words to say |
| `coined_by` | The owner, the persona, or both |
| `source` | Chat and turn range, as memory rows carry it: the deletion key. One chat; see §7 on forgetting |
| `origin` | Memory's three values: the owner's (an item, meaning or form the owner wrote or edited, which no later pass may write over), model-written from a clean stretch, or model-written from an untrusted one, by the recorded taint of the stretch. A turn no checkpoint covers is unknown, and unknown loads as untrusted, as memory's does |
| `due_at` | For a thread or a plan: the time the introducing chat gave for it, if it gave one. Resolved by code from a typed date the pass returns, against the turn's clock (`clock.rs`) in the owner's IANA zone; a date the pass cannot type leaves it empty |

An event is about a **subject**: an item, or a memory record by its `uid` where no item covers
it. An item names the memory `uid`s it was made from, so a row and its item are one subject.
Each event carries chat, turn ordinal and time:

- `introduced`: the item first appears, and who brought it.
- `cued`: the owner's words reached it, with the retrieval's score and how many other items
  the same cue reached.
- `offered`: it rode in a note to the model, and in what form.
- `said`: the persona's reply referred to it, the form used and its length in words.
- `taken_up` / `passed` / `repaired`: what the owner's next turn did with it.
- `withheld`: it was eligible and deliberately not offered (§8.2).

**What changes is derived, never stored.** How far an item is shared (said, acknowledged, taken
up by the owner), whether a thread has been raised or closed and whose move it is, and an
item's activation are all folds over the event log; none needs a model. So when a chat is
forgotten and its events go, nothing an erased event justified is left standing on the item.
Activation is a pure function of the event log and the clock:
it rises with a mention or a cue and falls with turns and with days. Because it is a function,
changing its shape re-reads the same log rather than migrating a store, and a replay can compare
two shapes on one recorded chat.

**A turn's events land with its reply.** A persona turn can fail after its notes were built,
and today the ledger is put back (`told_before`) so that an offer nobody read is not an offer
spent. A durable log cannot be put back, so a turn's `cued` and `offered` are appended only
once its reply is recorded. The transcript and `memory.db` are two stores, so this is an
order and not one write: reply first, then events, and a crash between the two loses that
turn's events, which is the safe side. A turn that is rolled back leaves no event, and
no fold or readout counts an offer nobody saw.

**A closed set on an append-only store is a wire format.** An item kind or an event the code
does not know loads as unknown and is skipped by every fold; it never fails the record.

**Who writes.** In a chat, code appends events only: which items were cued and offered is known
to the harness, and whether the reply said one is read from the reply against the item's handle
forms. Items themselves are written by the nightly writer's quarantined pass
(`persona::writer`), which already reads the chat with its per-turn taint: it proposes items
and handles from a stretch, under the rules memory rows follow (an untrusted stretch may add
candidates and never clear or raise anything). The owner sees and edits items where memories
are curated.

## 5. Four behaviours

### 5.1 Told

`today::Ledger` generalised and made durable: any item the persona has said is known to be
said, across restarts and chats, from the `said` events. An item already said and acknowledged
is not offered as news again. Where the record is uncertain (said, and no sign it was taken
in), the persona may ask whether it has mentioned it, which is what people do with their own
poor memory for whom they told (§2.1). That question reads only `said` events from clean
chats: a mark from a tainted chat silences and never prompts the persona to speak, or outside
content could choose which shared item it asks about.

### 5.2 Cued

Per-turn recall stays keyed to the owner's words. Between retrieval and the prompt goes a gate.
**It governs the whole per-turn fold, memory rows included**: `recall::per_turn`'s three rows
are where today's over-mention comes from (§2.6), so a gate over items alone would leave it
untouched. A retrieved row is a subject like any other, by its item where it has one and by its
`uid` where it does not, and it passes the same tests. Nothing is lost that the persona can
still reach: `memory_search` remains its own to call.

**Chat start is the other door, and it is not gated at step 2.** `recall::chat_start` is
material, stored once: what the persona needs to know going in, chosen by budget and recency.
It stays that, with two changes. Its rows are recorded as events with their door named, so
step 2's readout counts chat start apart from the per-turn fold and does not credit the gate
with a fall it only half produced. And when threads are switched on (§9, step 4) the
`(Left open: …)` text leaves the chat-start block, since a thread offered once and closed when
passed cannot also ride as text at every chat's opening. Whether chat-start rows already said
and acknowledged should be thinned is left until that readout exists.

- **Distinctive.** A cue that reaches many items offers none (§2.4).
- **One.** At most one item is offered for a reply.
- **Rested.** An item said recently is not offered again until it has rested; the rest
  lengthens with each use that got no uptake, and an item passed over repeatedly is retired.
- **Background is not an offer.** What the persona needs in order to *understand* the owner
  (they used a handle; here is its meaning) is material. What it might *raise* is the one
  offered item. The two are separate notes, so that knowing something stops pulling the reply
  toward saying it (§2.6).

When the owner uses a handle the ground cannot resolve, the persona asks. An outsider cannot
unpack a pair's shorthand (§2.2), and pretending to is the wrong recall that reads worst.

### 5.3 Compressed

The note that offers an item names the form to use, chosen from how far it is shared (derived, §4) and its
activation: the full description at first mention; the handle once it has been taken up; a
fuller reintroduction when it has been inactive for days. After a `repaired` event the form
steps back out. This is the behaviour models lack and the one §2.3 ties to closeness, and it is
directly measurable: an item's reference length across its mentions should fall the way §2.2's
curves do.

### 5.4 Returned to

- **Threads** (§2.5). An unfinished thread or a stated plan may carry a `due_at`; whether it has been raised or closed is derived (§4). It is offered once, at a
  chat's opening or a change of topic, never across what the owner is in the middle of saying.
  If the owner passes it, it closes.
- **Callbacks.** A callback is an item reused in a new context: the owner's current words cue
  it, it is not what the reply is otherwise about, and it has been taken up before. Items the
  owner coined or both built are preferred over the persona's own lines. Subject to the same
  gate as any offer. Callbacks begin switched off (§9, step 4): the upside is unmeasured and a
  mistimed one is the single memory behaviour shown to cost something (§2.6).

## 6. Where it attaches

- **Per turn.** `mecha-cli`'s persona turn assembles notes as `[memory_note, recall_block,
  anchor, call_note, today_note, variety_note, scene_note]`. The ground adds two: the
  background note and the offer. Each is its own note with its own clean and untrusted stems,
  registered where the others are (`Taint::arm_for_content`, `message::is_recorded_note`,
  `agent::is_harness_voice`). The computation lives in a core module beside `persona::today`.
- **The cached prefix is untouched**, and the turn is not free. Nothing per chat enters the
  system prompt, but a per-turn note makes the last turn re-prefill once it is projected off
  the wire, which `PERSONA-LIFE-DESIGN` §4.5 costed when the today-block made it two notes a
  turn. The ground makes it up to four. The offer is present only on a turn that has one, the
  background only on a turn where the owner used a handle, and step 2 measures the added
  prefill on the local model before it is left on.
- **The typed shown set.** The `offered` event is the record `PERSONA-LIFE-DESIGN` §4.5 already
  asked for beside `Record::Notes`: which items rode, not only which text.
- **D28.** Step 1 builds D28's log as ruled, and the ground's events are rows in it: every
  access to a memory record at chat start, per turn, by `memory_search` and by `memory_read`,
  keyed by `uid`, with the harness's doors counted apart from the persona's own calls, beside
  the owner's curation acts. `cued` and `offered` carry the `uid`s they came from, so D28's
  report reads off the same log and the ground adds what was said and how it was received. D28 also ruled that by default nothing in the log changes what is
  recalled, "because use-driven strengthening is how a persona fixates". Memory ranking keeps
  that. **The gate of §5.2 and the form of §5.3 are a departure from it**: they read the log to
  decide what is offered and how. The direction is the opposite of the one D28 feared, since a
  use makes an item rest and never makes it likelier, but it is an exception to a recorded
  ruling and is the owner's to make (§10, decision 8). Step 1 of §9 records only and needs no
  exception.
- **Memory v2.** Not a dependency. The ground reads persona memory as it is. If personas later
  move onto memory v2, items map onto its objects and the event log onto its exposure record;
  that move is a separate decision.

## 7. Rules it must keep

- **No persona text in the repository.** Tests use made-up items; measurements keep numbers.
- **Owner files are never rewritten.** Items are machine-written and owner-curated, as memory is.
- **Forgetting is complete.** `persona::memory::forget_chat` erases every item sourced to a
  chat, and every event from that chat. An item has one source, the chat that introduced it,
  and it goes with that chat, events from other chats included: D27's interim rule, kept so the
  erase stays deterministic and no model runs inside it. Its `meaning` is written once, by the
  introducing chat's pass, and never sharpened from a later chat. Its forms do accrue as the
  pair shortens them, so each form carries the chat it was first used in: forgetting that chat
  drops the forms it coined, and the reference steps back out to the longest form left. If the
  pair brings a forgotten thing up again later, the writer makes a new item from that chat.
  Both erases also cascade by record: an item made from a forgotten memory row goes with it,
  `meaning` and forms included, whether the row went with its chat (`forget_chat`) or alone
  (the owner's single-record `persona::memory::forget`), since an item sourced to one chat may
  cite a row written from another. The match is over every `uid` in the record's lineage
  (`Memory::lineage`), so an item made before a row was corrected is found; and the record's
  events are erased as D28 rules. `Memory::lineage` is private to `persona::memory`, which is
  where the ground's erase lives.
- **Withdrawing is honoured, not only deleting.** A row can leave recall without being
  deleted: invalidated (`Memory::invalidate`), unshared (`Shared::unshare`), or no longer
  visible to this persona after its groups or the row's audience narrowed. Recall honours each
  at once, so the ground does: an item is ineligible, as background and as an offer, while
  every row it was made from is invalidated, not visible, or gone. That is derived at the time of
  use, so it needs no migration and a row shared again brings its item back. And the erase does not fail soft: `forget_chat` reports a
  scene it could not remove and carries on, but an item that outlives its chat keeps riding in
  prompts. The ground is in `memory.db`, so its erase is part of the same transaction as the
  rows': a failure refuses the whole forget before anything is deleted, where a store erased
  afterwards could only report an error for a forget that had already happened.
- **Incognito writes nothing.** An incognito chat reads the ground and stores no event. It
  still has events: they are held in process for the chat's life, folded like any others, and
  dropped with the room, which is how `PersonaSession::told` behaves today. Without them the
  told fold is empty on every turn and each line rides every reply, the failure the ledger was
  built for.
- **Taint.** An item from an untrusted stretch is a candidate, as an untrusted memory row is:
  it is stored and enters no prompt, neither as background nor as an offer, until the owner
  approves it, and an approved one rides under the untrusted stem. Its `meaning` is model-written
  prose about outside content, and the ground is per persona, so it would otherwise surface in a
  chat that never saw the stretch it came from. Origin is per form as well as per item: a form picked from an
  untrusted stretch is a candidate form on a clean item, entering no prompt until approved, and
  an approved one makes the offer that names it ride the untrusted stem. A tainted stretch can
  mark an item said; it cannot mark one taken up. Because that makes every tainted mark a use
  with no uptake, the folds that lengthen an item's rest, retire it or raise its activation
  ignore a `said` from a tainted chat: such a mark quiets the item in that chat and nowhere
  else, and never moves the form it is referred to by. In a clean chat any item whose form the reply uses is marked
  said, offered or not, so a spontaneous re-telling is seen. That match needs the floor the cue
  gate has, since a short form will collide with ordinary wording and a false mark is durable,
  crosses chats and feeds the folds that rest and retire: a form counts only if it is
  distinctive (it reaches this item and no other, and is not made of common words alone); for
  an item that was not offered this turn only its longer forms count. In a tainted stretch marking said
  is a way to silence an item, so there it is bounded as
  `PERSONA-LIFE-DESIGN` §4.5 bounds `told`: only a subject this chat's notes recorded as
  offered or as background, matched against that recorded id list and never against a note's
  text, and any other id is refused.
- **Recall stays keyed to the owner's words**, never the persona's.
- **Nothing crosses personas**, and nothing reaches the knowledge graph. One existing route
  does carry another persona's row into a chat: a fact the owner shared rides in the chat-start
  block (`Shared::visible_to`), stays in the history, and is read by the writer. The item pass
  makes nothing from it: a candidate that would cite a shared copy is dropped. What the pair
  goes on to say about a shared fact in their own chat is theirs, and an item for it is sourced
  to that chat and cites no row. The alternative ties an item to the shared copy and cascades
  the copy's erase into every persona's ground; those are separate `memory.db` files beside
  `shared.db`, which cannot share one commit, so a failed cascade would leave exactly the item
  that outlives its row. It would need an idempotent erase retried until it completes, with
  the item ineligible meanwhile under the withdrawal rule above. That cost is why the simpler
  rule is proposed; the choice is the owner's (§10, decision 9).
- **Lock is `PERSONA-DESIGN` §9.9's, unchanged.** Locking hides the chat; the writer still
  reads it, and its items behave in other chats as its memory rows do.

### Out of scope

- Moving personas onto memory v2, and anything in the assistant's own chats.
- Voice calls: the ground is read there as in any chat, and no behaviour here is tuned for speech.
- Anything shared between personas, and anything written to the knowledge graph.
- Changing how memory rows are ranked (D28's default stands for ranking).
- A model that learns to compress; the form is chosen by the harness.

## 8. The instrument

### 8.1 What can be read from the log

Offline, through `mecha persona measures`, as counts and distributions and never as text:

- **Re-telling:** how often an item is said again after it was acknowledged, by lag.
- **Compression:** an item's reference length over its mentions, against the human curves
  (the released repeated-reference corpus of Hawkins et al. 2020 gives a reference shape).
- **Insider rate:** the share of turns that use a handle, by how long the pair has talked, and
  the spread of item kinds (the entropy measure of §2.3).
- **Unsaid:** the words of `meaning` behind the handles a turn used: how much went without
  saying.
- **Cue to mention:** how often a cued item was offered, said, and taken up; the distance in
  turns and days between an item's introduction and each return to it.
- **Threads:** opened, due, raised, closed, and what the owner did.
- **The dependency graph:** items as nodes, and an edge wherever one item's mention cued
  another. This is the structure the owner asked to be able to see.
- **The owner's own callbacks:** a handle the owner uses first in a chat. This is the outcome
  that needs no rating.

### 8.2 The experiment the literature lacks

§2.7's first question can be answered here. Where an item passes the gate, the harness may at
random withhold it, and record `withheld`. Comparing the turns after a used callback with the
turns after a withheld one estimates its effect, on the behavioural outcomes above and on a
rating if the owner gives one. The same design tests compression (handle against full form)
and follow-up timing. It is off unless the owner turns it on (§10).

### 8.3 Before any of it is fitted

`PERSONA-CONTEXT-DESIGN` §8's gates apply: a positive control, a panel of at least three
personas, and offline measurement first. Persona replay uses recorded notes, so it cannot show
what a new note would have done; step 2 below adds a replay mode that recomputes the ground's
notes at the branch point.

## 9. Build order

1. **Record only.** The store, the event log, `introduced`/`cued`/`offered`/`said`/`taken_up` derived
   from chats as they happen, and the nightly item pass. No prompt changes. Read the baseline:
   today's re-telling rate, reference lengths, insider rate, and how often the open threads
   that already ride in recall are raised. This is D28 as ruled (§6) and the typed shown set,
   built once.
2. **Told and the gate.** `today::Ledger` moves onto the store; the whole per-turn fold goes
   through the gate, rows and items alike; background and offer become separate notes. This is
   the largest change to what a persona is told, since most of today's per-turn rows will not
   pass, and it is measured on replay before it is switched on. The readout counts what the
   gate displaces as well as what it removes: the persona's own `memory_search` and
   `memory_read` call rates are measured beside re-telling and over-mention, since recall that
   moved from the owner-keyed fold to the persona's own calls is not a fall in over-mention and
   would erode the rule that the owner's words key recall. Replay mode for recomputed notes.
   Measured against step 1's baseline: re-telling and over-mention should fall.
3. **Compression.** The offer names the form. Measured: reference length over mentions.
4. **Threads, then callbacks.** Each behind its own switch per persona, each measured before it
   is left on.
5. **The experiment** (§8.2), if ruled in.

Each step is useful alone, and step 1 changes nothing a persona says. It is not invisible:
the nightly pass writes items, and the owner sees them where memories are curated.

## 10. Decisions for the owner

1. **A connection measure.** Behavioural outcomes only (the owner's own callbacks, reply length,
   return), or also an occasional one-tap rating after a chat? A rating is the closer analogue
   of §2.3's measure and it changes the conversation it rates. Proposed: outcomes, with the
   rating available and off.
2. **The withholding experiment** (§8.2). It makes some chats flatter by design. Proposed: built
   at step 5, off until asked for.
3. **Who writes items.** The nightly writer alone (proposed: one place a model authors, already
   quarantined), or also the chat as it goes, which would make a handle usable the same day at
   the cost of a model writing shared state mid-chat.
4. **Human-like forgetting.** People forget most of a conversation and hedge. Should a persona
   hedge about what it told the owner, or use its exact record? Proposed: exact for what it
   knows, hedged only where the record is itself uncertain (§5.1).
5. **Where the instrument's readouts live.** D28 named `mecha persona memory report` and the
   curation page for its log. Proposed: D28's report stays there for memory records, and the
   ground's readouts go through `mecha persona measures` beside the repetition measures they
   extend, with the graph view a later page. That is a second surface over one log, and is the
   owner's to accept or fold into the first.
6. **Memory v2.** Build on persona memory as it is (proposed), or wait for personas to move.
7. **Who authors a handle.** The writer's pass choosing among spans of wording the chat
   actually used (proposed: §2.2 says which words survive, and it keeps a model from coining
   the pair's shorthand), or free to write one.
8. **An exception to D28** for the gate and the reference form (§6): the log deciding what is
   offered, in the direction of resting an item after use and never of strengthening it. That
   holds for what is offered. It does not hold for the form: an item used more is referred to
   more briefly, and a mention raises its activation, which is strengthening-shaped and is the
   half of the exception D28's wording touches. Needed from step 2 of §9 (the gate) and step 3
   (the form); step 1 needs none.
9. **Items from shared facts.** The item pass makes nothing from a fact the owner shared across
   personas (proposed: no erase then has to reach another persona's store), or such a fact can
   become an item in each persona's ground, tied to the shared copy, with the retried erase §7
   describes. The first loses the pair's shorthand for a shared fact until they talk about it
   themselves.

## Sources

Bell & Healey 1992, *Human Communication Research* 18(3). Berntsen 2009, *Involuntary
Autobiographical Memories*. Brennan & Clark 1996, *JEP: LMC* 22(6). Brown-Schmidt & Benjamin
2018, *Policy Insights from the Behavioral and Brain Sciences* 5. Ariel 1990, *Accessing
Noun-Phrase Antecedents*. Chafe 1994, *Discourse, Consciousness, and Time*. Gundel, Hedberg &
Zacharski 1993, *Language* 69. Clark 1996, *Using Language*.
Clark & Brennan 1991, "Grounding in communication". Clark & Wilkes-Gibbs 1986, *Cognition* 22.
Cox, Lee & Ooi 2023, arXiv:2308.04879. Ghibellini & Meier 2025, *Humanities & Social Sciences
Communications* 12. Gopie & MacLeod 2009, *Psychological Science* 20(12). Hawkins, Frank &
Goodman 2020, *Cognitive Science* 44(6), arXiv:1912.07199. Hawkins et al. 2023, *Psychological
Review* 130(4), arXiv:2104.05857. Hopper, Knapp & Scott 1981, *Journal of Communication* 31(1).
Hu et al. 2026, arXiv:2601.13722. Hua & Artzi 2024, arXiv:2408.01417. Huang et al. 2017, *JPSP*
113. Jones et al. 2026, arXiv:2602.08208. Ray et al. 2018, *JPSP*. Rossignac-Milon et al. 2021,
*JPSP* 120(4). Schegloff & Sacks 1973, *Semiotica*. Schober & Clark 1989, *Cognitive
Psychology* 21. Sumida, Inoue & Kawahara 2026, arXiv:2608.24189. Templeton et al. 2024,
PsyArXiv g38cx. Tulving & Thomson 1973, *Psychological Review* 80. Wang et al. 2026,
arXiv:2606.08081. Yoon, Benjamin & Brown-Schmidt 2016, *Cognition* 154.
