# Persona context: what the model reads

**Status:** proposed 2026-10-05. Rulings pending (§7). Nothing here is built.

**The question:** why does a persona keep doing what the owner asked it to stop, chain edits of
its own pictures, run away inside one turn, and claim things it cannot do? And what is the fix at
the root, rather than one more note in the prompt?

**The answer in one line:** the model reads a context that is mostly the harness talking and the
persona's own past machinery, with the owner's words at 2–4% of it. The model continues whatever
dominates its context. Each symptom below is that one behaviour. Each earlier fix added more text
to the same context, so each one also added to the cause.

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
   each, around 8 minutes in all. It stopped only because the run hit `max_turns = 12`.
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
  (`persona::call::note`), the variety note (`persona::variety`), the identity reminder
  (`persona::safety`) and the memory block (`persona::recall`). Because they live in the message,
  they stay in the history and are re-sent on every later turn.
- **Tool results** carry guidance as well as facts. An image result is about 900 characters:
  where the file is, not to overwrite the original, "To change it further, edit … next", and the
  near-copy notice with its recovery advice (`imagegen.rs`).
- **A call's barge-in** (`persona_chat::speak`) cancels the run in flight (`CancelReason::Stopped`).
  `image_generate` honours that cancel and stops the ComfyUI job, because the chat's Stop button
  needs exactly that. The result is "Cancelled — the generation was stopped and nothing was
  saved." (`ARCHITECTURE.md` §Interruption and steering says tools are never interrupted
  mid-call; the image tool is the exception.) Steered owner speech is folded in after that result.
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

1. **Guidance for one turn is stored as history.** Notes and the memory block are written into the
   owner's message, so they accumulate (§3.1).
2. **Tool results carry instructions as well as facts**, and those instructions accumulate too.
3. **Interrupted work stays in the history.** A turn the owner spoke over keeps its reasoning, its
   call and a "Cancelled" result. The model reads that as unfinished work (L1/L2 at M2).
4. **A slow side effect runs inside the conversational turn.** A ~40 s picture is a synchronous
   tool call, so:
   - the turn blocks;
   - a barge-in kills the render;
   - one turn can chain pictures up to `max_turns`.
5. **The persona keeps its look by editing its last picture.** Edits cannot change a pose, so new
   poses come back as near-copies and drift across chains. #569's face anchor treats the drift.
   The cause is that the persona edits at all when the owner did not point at a picture.
6. **Guidance is detached from capability.** A note about a tool is sent whether or not the
   persona has the tool.

## 5. Design

Each change is a rule about the context or the conversation, not a new prompt.

1. **Ephemeral guidance.**
   - Harness notes and the memory block go on the **current request only**, after the history.
     They are never written into a message, and the transcript records them in their own record,
     so nothing is lost for audit.
   - The history the model sees is the conversation: owner words, persona replies, delivered
     pictures.
   - Cache: the history is byte-stable between turns, so it stays a prefix of the next request.
     Only the tail is new.
2. **Factual tool results.** A result says what happened (path, edit or new, time; for an edit,
   the similarity). How to use the tool, and what to do next, lives **once**, in the tool's
   description.
3. **The history is what reached the owner.** A turn the owner interrupted leaves its delivered
   words in the history and nothing else: no reasoning, no unanswered call, no "Cancelled"
   result. The transcript keeps the full record. This applies to every tool, not only pictures.
4. **Pictures are jobs, not turns.**
   - `image_generate` queues a job and returns at once ("being made").
   - The picture is delivered into the conversation as an event when it is ready: the owner sees
     it, and the model is told at its next turn.
   - **One job per conversation at a time**: a second request while one is pending is refused
     structurally.
   - A barge-in cancels the *talking*, never the job. The owner can still stop a job explicitly
     (the Stop button, or a call's "stop").
   - This removes the call's 40 s silence, the cancel-and-retry loop, and the in-turn runaway.
5. **Edits only on the owner's initiative.**
   - `reference_images` in the persona form accepts only a picture the owner pointed at in the
     turn being answered: the Edit button's message, a picture they attached, or one they named.
   - Every other picture is new, with `cast` carrying identity (the persona's linked character,
     §8.6 of `PERSONA-DESIGN.md`).
   - This is the owner's stated intent (2026-10-05). It also removes edit-chain drift at its
     source.
6. **Guidance travels with capability.** A note about a tool is part of that tool, its description
   or its own contribution to the call note, so a persona without the tool never hears about it.
   (The call note's `PICTURES` sentence is the instance.)

## 6. What these make unnecessary

Each of these was right for its symptom. Under §5 the symptom has no cause left. Retire each one
only with a before/after measurement on the replay (§8) and on the repetition harness
(`persona/variety`'s echo readings), never by assumption:

- the near-copy notice's recovery advice and the "To change it further, edit … next" line (§5.2,
  §5.5);
- the variety note written into each turn (`persona::variety`; its guidance becomes ephemeral under
  §5.1, so whether it is still needed is a measurement);
- the repeat guard's retry wording around repeated new pictures (#543), under §5.4's one-job rule;
- `PriorThinking::Drop`'s tool-turn exemption, re-examined once §5.3 removes interrupted tool turns.
  The 2026-08 empty-call finding is still to be honoured.

## 7. Rulings needed

- **R1. Ephemeral guidance (§5.1):** the notes move out of the stored conversation.
- **R2. The history is what reached the owner (§5.3)**, for every tool.
- **R3. Pictures as jobs (§5.4).** The visible change: the persona answers before the picture
  arrives, and one picture is in flight at a time.
- **R4. Edits only on the owner's initiative (§5.5)**, with `cast` for every other picture.
- **R5. Retire superseded fixes by measurement (§6).** Each one is removed only when the replay and
  echo readings show no regression.

## 8. Build order and measurement

- **Order:** §5.1 and §5.2, then §5.3, then §5.6, then §5.5, then §5.4. The first three are what
  L7 measured; §5.6 is small; §5.5 and §5.4 change behaviour the owner sees.
- **Gate for each step:** rerun the replay. L0 must move toward L7 for M1 and M2, with no empty
  replies. Read the echo/repetition readings before and after. §5.4 and §5.5 each need a new
  measurement:
  - picture latency and loops on a scripted call;
  - the near-copy rate and pose success of new-with-cast against edit, scored with ArcFace as in
    `IMAGE-COMPILER-RESEARCH.md`.
- **Harness:** `~/.mecha/research/persona-context-2026-10-05/` on the operator's machine, mode
  0700, never committed, because it reads real sessions.
  - `replay_live.py` is the live-faithful replay, and `replay_lean.py` holds L6/L7.
  - `persona-a-tools.json` is the dumped tool definitions.
  - Its `README.md` says how to rerun. The router must serve the persona's model with nothing
    holding it.
