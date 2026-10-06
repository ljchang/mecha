# Persona context: what the model reads

**Status:** accepted 2026-10-05 with the amendments below (§7). Being built in the order in §8.
§5.4 (pictures as jobs) is a separate build. It is built against the run notes of §5.1.

**The question:** why does a persona keep doing what the owner asked it to stop, chain edits of
its own pictures, run away inside one turn, and claim things it cannot do? And what is the fix at
the root, rather than one more note in the prompt?

**The answer in one line:** the model reads a context that is mostly the harness talking and the
persona's own past machinery, with the owner's words at 2–4% of it. The model continues whatever
dominates its context. Each symptom below is that one behaviour. Each earlier fix added more text
to the same context, so each one also added to the cause.

**Why the fix lives in core:** the harness has no place for its own words except the owner's
message. Everything after that is repair work: a registry of text prefixes to tell harness text
from the owner's (`agent::is_harness_voice`, about twenty entries, six of them a persona's), and
wire views that remove some of it
again on the way out (`PriorNudges`, beside `PriorThinking` and `PriorTails`). The assistant's
chat adds the calendar reference and the situation brief to user turns the same way. Personas show
the cost first because their chats are long, mostly talk, and run on a small local model. So the
context rules in §5.1–§5.3 are built in `mecha-core`, for every conversation. They are not built
as one more persona patch in the serve layer.

Privacy: this document carries no conversation text, no persona names and no session ids. The
personas are A (has `image_generate`) and B (does not). The measurements are counts and
character totals. The replay harness that produced them reads real sessions, so it lives outside
the repository (§8).

---

## 1. Symptoms (2026-10-05)

1. **A voice-call picture loop (persona A).** In one call, the persona asked for a picture over and
   over. 9 turns were stopped by the owner speaking and 7 pictures were cancelled mid-render. After
   the owner asked it to stop sending pictures, it called `image_generate` four more times.
2. **A runaway inside one turn (persona A, text chat).** One turn made 12 picture calls, about 40 s
   each, around 8 minutes in all. It stopped only because the run hit `max_turns = 12`. That cap
   was an operator override in `[agent]`, not the repository default of 40. The owner ruled
   against a low cap (§7, R6) and it is gone. The runaway is fixed structurally by §5.4's
   one-job rule, not by a turn cap.
3. **"The same picture again" (persona A).** 75% of the persona's edits came back as near-copies of
   their reference, both before and after the 2026-10-04/05 image changes (§3.4).
4. **Edits nobody asked for (persona A).** Of the persona's picture calls on 10-05, 77% were edits
   it chose itself. The owner expects a new picture unless they point at one (§3.5).
5. **A claimed picture (persona B).** On a call, a persona with no image tool said it had sent a
   picture. No tool was called.

## 2. How a persona request is built today

- **History:** the session's messages as recorded. Assistant turns are sent with their reasoning
  (`reasoning_content`, `provider::openai::encode_message`). `PriorThinking::Drop`
  (`message.rs`, #517) removes earlier reasoning only from *plain replies*. A turn that called a
  tool keeps it, because stripping it in 2026-08 produced bare, empty tool calls (6/6).
- **Harness notes** are written *into the owner's message* for the turn: the call note
  (`persona::call::note`), the variety note (`persona::variety`), the edit note
  (`persona::edit`), the identity reminder (`persona::safety`) and the memory block
  (`persona::recall`). Because they live in the message, they stay in the history and are
  re-sent on every later turn. `PriorNudges::Drop` takes the stale variety and edit notes back
  out at send time. The rest stay.
- **Tool results** carry guidance as well as facts. An image result is about 900 characters:
  where the file is, not to overwrite the original, "To change it further, edit … next", and the
  near-copy notice with its recovery advice (`imagegen.rs`).
- **A call's barge-in** (`persona_chat::speak`) cancels the run in flight (`CancelReason::Stopped`).
  `image_generate` honours that cancel and stops the ComfyUI job, because the chat's Stop button
  needs exactly that. The result is "Cancelled — the generation was stopped and nothing was
  saved." It is one of the tools a cancel interrupts mid-call. `ARCHITECTURE.md` §Interruption and
  steering keeps that set. Steered owner speech is folded in
  after that result.
- **Persona chats are unseeded** (`setup::persona_provider_config`, `PersonaUse::Converse`). The
  session's `config` record shows the provider's `seed = 42` *as written*, not what is sent.

## 3. Evidence

### 3.1 What the model reads

The wire as sent (live-faithful encoding, §3.2), split by source, for persona A's voice-loop call:

| source | voice-loop moment (M1) | end of the runaway turn |
|---|---|---|
| owner's words | **3.5%** | **2.4%** |
| harness notes | 39.0% | 26.7% |
| persona's reasoning (tool turns) | 23.1% | 23.8% |
| tool results | 12.4% | 27.3% |
| tool calls | 6.6% | 10.4% |
| persona's replies | 7.0% | 4.3% |
| system prompt | 8.4% | 5.2% |
| total | 32,852 chars | 53,436 chars |

The harness notes at M1 break down as: 3 copies of the memory block (7,486 chars), 18 "From the
harness" notes (4,325), and 2 identity reminders (988). All of them are re-sent on every turn.

The two M1 totals, 32,852 here and 32,941 in §3.2, are two captures of the same moment. Each
percentage is of its own row's total.

### 3.2 The replay

**Method:**
- Two moments of the voice loop are replayed against the live model, through the router, with the
  model resident. **M1** is the turn just after the owner asked the persona to stop sending
  pictures. **M2** is a later retry in the same loop.
- **Encoding** matches the wire: `PriorThinking::Drop`, no seed, `reasoning_budget_tokens = 1024`
  (`SPOKEN_THINK_BUDGET`), and temperature 0.6.
- **Tool definitions** are the persona's real ones, dumped from `persona::agent::registry_as` by a
  throwaway test.
- **The measure** is whether the reply calls `image_generate`. Eight unseeded samples per cell.

| variant | M1 | M2 |
|---|---|---|
| L0 the live wire | 6/8 | 8/8 |
| L1 interrupted picture turns removed (reasoning, call, "Cancelled" result; owner text folded) | 7/8 | 1/8 |
| L2 only the reasoning of interrupted turns removed | 6/8 | 0/8 |
| L4 all conversational reasoning kept + L1 | 8/8 | 1/8 |
| L5 all earlier reasoning dropped, tool turns included | 7/8 | 7/8 |
| L6 lean context: past turns keep only the owner's words (notes on the current turn only); tool results cut to their factual first line | 7/8 | 3/8 |
| **L7 lean context + L1** | **1/8** | **0/8** |

- No variant produced an empty reply.
- Under L7 the context is 13,245 characters instead of 32,941. The owner's share rises from 3.5% to
  8.7%, and the harness share falls from 38.9% to 3.1% (M1).
- **Neither L1 nor L6 is enough alone at M1; together they are.**

An earlier run, the V-series, is superseded:
- **Not live-faithful.** It kept the reasoning of plain replies and sent fixed seeds.
- **One bad cell.** Its V4 cell was void, because the composition of its two transforms undid one
  of them.
- **One unreproduced result.** Its striking cell (reasoning stripped from cancelled turns: 0/7 at
  M1) did not reproduce under close live-faithful variants (L2, L4).
- It is kept in the harness outputs for the record (§8).

**Limits:**
- Two moments from one call, eight samples per cell. That is strong for direction, not for exact
  rates.
- **L6 removed more than §5.1 does.** It dropped every parenthesised harness block from past
  turns, including the files block, the memory block and the session goal. It did not move them.
  The build keeps the files and the goal in the first turn and moves memory into the run's notes
  (§5.1). So L7 is the direction, not a measurement of the build. Each step's gate replays the
  projection *as built* (§8).
- The replay tests what the model *reads* (changes 1–3 in §5). Changes 4–6 alter what *happens*,
  so they cannot be replayed and each needs its own measurement once built.

### 3.3 Ruled out

- **The seed.** Persona chats are unseeded (#517). Today's `seed = 42` in the record is the
  provider config as written.
- **#569 (face anchor) and #560 (int8 model).** The near-copy rate is unchanged across both:
  - #569: 75% of edits before it (77 edits), 73% after (15).
  - #560: 73% before it (60), 78% after (32).
  - #560 halved the median edit time, from 83 s to 40 s. Slowness is the number of pictures per
    turn, not the time per picture.
- **The wording of the cancellation result:** 4/7 and 4/7 against a 5/7 and 4/7 baseline in the
  V-series.
- **The balance of visible reasoning** (L4/L5): no variant moved M1.

### 3.4 Edits and near-copies

- An edit's result flags a near-copy when its layout similarity to the reference passes the
  threshold (#408). On persona A, 75% of edits are flagged, at a median similarity of 0.96.
- The flagged edits are pose and arrangement changes, which `IMAGE-COMPILER-RESEARCH.md` already
  measured the edit model as weak at.
- The recovery advice in the notice ("If they ask: edit [the original] rather than this result")
  sends the next attempt back to the same reference.

### 3.5 Who starts an edit

Every `image_generate` call by persona A, classified by the owner turn before it:

| day | new picture | edit the owner started (Edit button) | edit the persona chose |
|---|---|---|---|
| 09-30 | 167 | 3 | 10 |
| 10-03 | 38 | 26 | 2 |
| 10-04 | 12 | 15 | 18 |
| 10-05 | 6 | 1 | **23** |

`IMAGE-COMPILER-RESEARCH.md` measured that identity lives in the cast reference: a portrait pointer
holds 0.74–0.78 ArcFace, against 0.33 from a description alone. It also measured that stated pose
and wardrobe release what the reference would copy. **None** of the persona's self-chosen edits
on 10-05 used `cast`.

### 3.6 The claimed picture

`persona::call::note` ends every call note with the `PICTURES` sentence ("A picture you make appears
on the owner's screen during the call … say you sent it"), whether or not the persona has
`image_generate`. Persona B's reasoning followed that sentence.

## 4. Root causes

0. **The harness has no channel of its own.** A `Message` is the only thing a request is built
   from, so harness text goes into the owner's message and then has to be recognised and removed
   again. Causes 1 and 2 follow from this.
1. **Guidance for one turn is stored as history.** Notes and the memory block are written into the
   owner's message, so they accumulate (§3.1).
2. **Tool results carry instructions as well as facts**, and those instructions accumulate too.
3. **Interrupted work stays in the history.** A turn the owner spoke over keeps its reasoning, its
   call and a "Cancelled" result. The model reads that as unfinished work (L1/L2 at M2).
4. **A slow side effect runs inside the conversational turn.** A ~40 s picture is a synchronous
   tool call, so:
   - the turn blocks;
   - a barge-in kills the render;
   - one turn can chain pictures until the run's turn budget runs out.
5. **The persona keeps its look by editing its last picture.** Edits cannot change a pose, so new
   poses come back as near-copies and drift across chains. #569's face anchor treats the drift.
   The cause is that the persona edits at all when the owner did not point at a picture.
6. **Guidance is detached from capability.** A note about a tool is sent whether or not the
   persona has the tool.

## 5. Design

Each change is a rule about the context or the conversation, not a new prompt. The general shape:
**every component that has something to tell the model (a tool, call mode, the safety layer,
memory) says it only while it is active, and in one of two places.** The stable prefix (the
system prompt or a tool's description) holds what is true for the whole chat. The run's notes hold
what is true for this run. Nothing a component says is written into the conversation.

### 5.1 Run notes

- **What they are.** `RunContext::notes` is the harness's text for one run. The loop adds it to
  every request of that run and never writes it into `Conversation::messages`.
- **Where they go on the wire.** The notes are extra text blocks at the end of the request's
  **last user message**, added at send time after every other wire view. They are never a message
  of their own, because two user messages in a row are invalid. On a run's first request the last
  user message is the owner's turn. After a tool round trip it is the message carrying the tool
  results, which is the slot steering already uses.
  - In that slot the notes come after the owner's steered words, as they come after the owner's
    words on a first request. That is the order L6 and L7 measured. A steered turn itself is
    unmeasured. If the owner's words are better last there, it is a change to `attach_notes`
    alone.
- **Why the end, not the owner's turn.** The notes move to the newest message with each request, so
  each request re-reads only the notes and the step before them. A note pinned to the owner's
  turn would be cached for the run, but the next turn would then re-read the whole of the
  previous run from the point where the note no longer is. That costs most after a tool-heavy turn
  and lands on a spoken reply's latency. At the end, the next turn's history is a prefix of what
  the server already holds, up to the previous reply.
- **The views run on the recorded history.** `message::answering` (the turn being answered,
  which `PriorThinking::Drop` cuts at) is found before the notes are added, so a trailing note
  never moves the cut.
- **Accounting.** `Agent::wire_bytes` counts the notes, so a pressure reading and the request
  describe the same bytes.
- **Cache breakpoints.** `CompletionRequest::trailing_notes` says how many trailing blocks are
  notes. A provider that marks a moving cache breakpoint (`provider/anthropic.rs`) puts it on the
  last block *before* them, because a write on a note is never read back.
- **Taint.** `Taint::arm_for_content` cannot see a note, because it reads `messages`. So the loop
  arms from `RunContext::notes` itself at run start, before the first request, by the same rule
  (`Taint::arm_for_notes`). Missing it would silently un-arm `private_data` in exactly these chats,
  with every test over `arm_for_content` still green. Memory of the owner arms `private`, and
  memory first read from outside also arms `untrusted`. Taint stays a property of the conversation
  and is recorded as before, so a later turn without the note stays armed. A subagent never
  inherits its parent's notes: they speak to the run that set them. The transcript is also
  how a torn taint record is re-derived, and the notes are no longer in its messages. So
  `Session::read` arms from the recorded notes (`Record::Notes`) by the same rule.
- **The record.** The session records each run's notes in their own record (`Record::Notes`). A
  build from before it skips the line, as with `Record::Extend`.
  - Any reader that rebuilds a moment to send it again must re-attach that moment's notes, or the
    replayed moment is not the recorded one. That covers `replay::extract` and
    `counterfactual::followup_branch`, which resubmit `messages` as recorded.
  - Neither serves persona chats today (a persona agent renders no learned rules). §8's gate
    replay is the external harness, which applies the projection itself.
  - Re-attaching notes is owed before either reader is pointed at a chat that carries them.
- **Guidance and material.**
  - *Guidance* changes per run and becomes notes: the call note, the variety note, the edit note,
    the identity reminder, and memory: the chat-start block and the per-turn recall.
  - **Memory and recall are two notes, never joined.** `recall::stem_of` reads only a note's
    leading stem. A clean chat-start block joined ahead of a recall from outside would arm
    `private` and never `untrusted`. That is why `arm_for_content` has two stems, and a test
    caught exactly this in #572.
  - *Material* is read once and kept: the files block stays in the first turn, where it is cached
    and compaction keeps it. The session goal stays too, because it is the owner's words.
  - The call note goes on **every** spoken turn. It no longer persists, so "first spoken turn of a
    stretch" no longer applies.
- **Chats recorded before this.** Their stored messages already hold notes. The persona projection
  drops every recorded persona note from the history it sends: call, variety, edit, identity
  reminder and memory, but never files or the goal, and never a block whose removal would leave its
  message empty (an empty user message is a 400, as an empty assistant one is). That replaces
  `PriorNudges`'s stale-note logic and keeps its empty-message guard.
  - **The cost, once per old chat.** Dropping every recorded note makes the server's cache diverge
    at the first note, which is in the chat's first turn. So an old chat's first turn after the
    change re-reads its whole history: roughly 8,000 tokens for a long chat, about 4–5 s at the
    router's measured ~1,800 tokens/s, on a spoken reply if that turn is spoken. After that the
    history is stable and each request repeats the one before.
  - The re-read cap this retires (`NUDGE_REREAD_BYTES`, owner ruling 2026-10-04) bounded a cost
    paid on every turn. This one is paid once per chat, and it is the owner's to weigh against
    keeping the old notes on the wire for good.
  - `is_harness_voice` keeps its entries, because old transcripts are still read by the miner and
    the UI.

### 5.2 Factual tool results

A result says what happened: the path, edit or new, the time, and for an edit the similarity. How
to use the tool, and what to do next, lives **once**, in the tool's description.

### 5.3 The history is what happened, not what was attempted

- A turn the owner interrupted is sent as **its delivered words plus whatever its tools delivered**,
  stated as a fact. A tool that finished before the barge-in, such as a memory write, stays.
- **The test is whether a result carries content, not whether its call was cancelled.**
  - A cancelled call that delivered nothing goes, with its result and the reasoning that chose
    it. `image_generate` is the case: a cancel saves nothing.
  - A cancelled call that delivered part of its work stays, as that part. A cancelled
    `document_read` returns the pages it already transcribed, and a reply may already quote them.
    So does a persona's `file_read`, whose replies cite pages by number, and a subagent's partial
    run.
  - The tool says which it was, by a typed mark on its result. The projection never matches
    the result's wording.
- No message is ever sent empty: an empty message is a 400 everywhere. An assistant message left
  with no text and no completed call is dropped whole, which is `drops_thinking`'s rule for the
  same reason. So is a tool-results message that held only the cancelled result: a Stop-button
  cancel folds in no owner speech, so nothing is left in it. The owner's words around a dropped
  turn fold into one user turn, so two user messages never sit in a row.
- This is a send-time **projection** of the recorded history, like `PriorThinking`. It is not a
  `Rewrite` record. The transcript keeps everything.
- **Its place among the views.** It is the first view that removes whole messages, so it runs
  last among them, after `PriorNudges` and before the run's notes are attached. Any view that
  locates blocks by position must run on the history before it does.
  - Before #572, `PriorNudges` indexed the recorded history and its views' output in step, on the
    stated precondition that no view removes a message. #572 removes that computation along with
    the stale-note rule it served. The ordering still holds for any later view.
- **Its accounting.** `Agent::wire_bytes` gets its own subtraction for the messages it removes,
  beside its addition for the notes (#572). A pressure reading and the request it predicts must
  describe the same bytes.
- It applies to every tool, not only pictures.

### 5.4 Pictures as jobs (separate build)

- `image_generate` queues a job and returns at once ("being made").
- When the picture is ready, the owner sees it. The model learns of it through the next run's notes
  (§5.1), and the history gains a durable record that it was delivered. That record is **text
  only**: the model is told a picture reached the owner, never shown its pixels. `image_view`
  stays the separate step, and images stay user turns only.
- **One job per conversation at a time.** A second request while one is pending is refused
  structurally.
- A barge-in cancels the *talking*, never the job. The owner can still stop a job explicitly (the
  Stop button, or a call's "stop").
- Built as a core mechanism for any slow side effect, with pictures as the first user.
- This removes the call's 40 s silence, the cancel-and-retry loop, the in-turn runaway, and the one
  image tool from the set of tools a cancel interrupts mid-call (§2). The others stay in it.

### 5.5 Edits only on the owner's initiative

- `reference_images` in the persona form accepts only a picture the owner **pointed at, by a typed
  reference** in the turn being answered: the Edit button's message, a picture they attached, or a
  "reply to this picture" action. Nothing is parsed out of the owner's words.
- Every other picture is new, with `cast` carrying identity (the persona's linked character,
  `PERSONA-DESIGN.md` §8.6).
- A persona with no linked character draws itself from its description, as today. This rule adds
  no identity source it does not have. The tool's description says so, and linking a character is
  the owner's way to fix it.
- This is the owner's stated intent (2026-10-05). It also removes edit-chain drift at its source.

### 5.6 Guidance travels with capability

A note about a tool is part of that tool: its description, or its own contribution to a note. A
persona without the tool never hears about it. The call note's `PICTURES` sentence is the
instance: it is sent only to a persona whose registry holds `image_generate`.

## 6. What these make unnecessary

Each of these was right for its symptom. Under §5 the symptom has no cause left. Retire each one
only with a before/after measurement on the replay and the regression panel (§8), never by
assumption:

- `PriorNudges`'s stale-note logic and its re-read cap (§5.1: there is nothing stale to drop once
  notes are not stored; the projection of old chats drops all of them);
- `recall::carries_now`'s first-turn placement of memory, and the call note's
  "first spoken turn of a stretch" gating (§5.1);
- the near-copy notice's recovery advice and the "To change it further, edit … next" line (§5.2,
  §5.5);
- the variety note itself (`persona::variety`). Under §5.1 it is one note for one run, never a
  stack, which is the condition it was measured in. Whether it is still needed once the history
  is lean is a measurement;
- the repeat guard's retry wording around repeated new pictures (#543), under §5.4's one-job rule;
- `PriorThinking::Drop`'s tool-turn exemption, re-examined once §5.3 removes interrupted tool turns.
  The 2026-08 empty-call finding is still to be honoured.

Not retired: `is_harness_voice`'s persona entries (old transcripts carry those notes), and
`PriorTails` (a cut-off reply is what the owner heard, and the trim is about how the model reads
it, not about harness text).

## 7. Rulings (owner, 2026-10-05)

The owner accepted the design with the amendments made in this revision ("Revise the doc and then
get started").

- **R1. Run notes (§5.1):** accepted, built in core as `RunContext::notes`.
- **R2. The history is what happened (§5.3)**, for every tool, keeping completed effects.
- **R3. Pictures as jobs (§5.4)**, as a core job mechanism, built separately on §5.1's notes.
- **R4. Edits only on the owner's initiative (§5.5)**, by typed reference only, with `cast` for
  every other picture.
- **R5. Retire superseded fixes by measurement (§6).**
- **R6. No low turn cap.** "We don't need to set a max turns … if we do it should be much longer."
  The operator's `max_turns = 12` override is removed. Runs use the default of 40.

## 8. Build order and measurement

- **Order:**
  1. §5.1 run notes, with §5.6 (the call note is rewritten in the same place anyway);
  2. §5.2 factual tool results;
  3. §5.3 the projection of interrupted turns;
  4. §5.5 edits only on the owner's initiative;
  5. §5.4 pictures as jobs, separately, once step 1's notes have landed.
- **Gate for each step:**
  - Rerun the replay against the projection as built. L0 must move toward L7 for M1 and M2, with
    no empty replies.
  - Read the echo and repetition readings before and after.
  - Run a **regression panel** over at least three personas, one of them without
    `image_generate`. It covers repetition and copying (`persona::echo`), the picture loop, the
    claimed picture, and identity in self-portraits. One persona's chats are too small a sample
    to tune on (`PERSONA-DESIGN.md` §12.7).
  - §5.4 and §5.5 each need a new measurement:
    - picture latency and loops on a scripted call;
    - the near-copy rate and pose success of new-with-cast against edit, scored with ArcFace as in
      `IMAGE-COMPILER-RESEARCH.md`.
- **Harness:** `~/.mecha/research/persona-context-2026-10-05/` on the operator's machine, mode
  0700, never committed, because it reads real sessions.
  - `replay_live.py` is the live-faithful replay, and `replay_lean.py` holds L6/L7.
  - `persona-a-tools.json` is the dumped tool definitions.
  - Its `README.md` says how to rerun. The router must serve the persona's model with nothing
    holding it.
