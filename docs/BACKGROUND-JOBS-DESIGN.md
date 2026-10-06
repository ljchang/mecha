# Background jobs: a slow side effect that outlives the turn

**Status:** accepted 2026-10-05 (rulings in §7). It builds `PERSONA-CONTEXT-DESIGN.md` §5.4, which the owner ruled
as R3: "pictures as jobs, as a core job mechanism, built separately on §5.1's notes". It is built
after that doc's §5.2 and §5.5, which also edit `imagegen.rs`.

**The question:** how does a ~40 s side effect (a picture first) run without blocking the turn,
without being killed by speech, without chaining inside one turn, and without inventing a new
format every reader has to learn?

**The answer:** the tool call returns at once, its result saying the work is "being made". The
work runs as a job owned by the conversation, not the run. When it finishes, **the same call's
result arrives late**:
- the page sees the result update (a picture appears where the call is);
- the model's stored history has that result rewritten to the finished, text-only fact;
- the model is told on its next run, through a run note (§5.1).

Nothing new appears on the wire, in the transcript or on the page: a late result is the shape
every reader already understands.

---

## 1. What exists today (traced 2026-10-05, at `ec6a5a63`)

**One `image_generate` call:**
- `ImageGenerate::call` checks the input and claims the repeat guard (`claim`, PR #543, keyed per
  workspace).
- `backend.generate` submits to ComfyUI and polls `history/{id}`.
- `save` writes `images/<stamp>-<seed>.png`, then the near-copy check and `write_manifest` run.
- It returns `image: <path>\n…`.

**Cancellation:**
- `run_tools` never aborts a call from outside. But `image_generate` watches `ToolCtx.cancel`
  itself (`await_server`, `run`'s poll loop). On cancel it `abandon`s its ComfyUI job and returns
  "Cancelled — the generation was stopped and nothing was saved." That is the one exception to
  "cancellation never interrupts a tool call mid-call".
- `CancelReason` is `Parked | Stopped | Shutdown`, and **every** stop writes `Stopped`: a voice
  barge-in (`speak()` in both chats), the Stop button (`/cancel` in both chats), a model switch,
  a client disconnect, and a crisis pause. A tool cannot tell speech from an explicit Stop.

**Reaching the page:**
- `AgentEvent::ToolResult` becomes `WireEvent::ToolResult{id, preview}`.
- `picture.js` `pictureOf` draws a tool entry whose preview's first line is
  `image: images/<x>.png`.
- The call screen's pictures come from the same entries.
- Each chat session has an `events: broadcast::Sender<WireEvent>` that outlives runs. Background
  tasks already send on it (`PersonaChats::apply_verdict`; chat.rs's mode and notice).

**The transcript** is drawn from the conversation's messages (`chat::transcript_entries`), not
from records. A new `Record` variant would be invisible without endpoint and page work, and "a
`Record` variant is a wire format every reader … would have to learn" (`persona_chat.rs`).

## 2. Shape

### 2.1 A deferred tool output (core, `mecha-core`)

- A tool may answer `ToolOutput::deferred(now, job)`. `now` is the immediate result text. `job`
  is a boxed future yielding the finished `ToolOutput`, run with **its own** cancellation token.
- The agent loop hands the job to `RunContext::jobs` when the run has one (a `JobSink`, given by
  a chat host) and records `now` as the call's result.
- With no sink (the CLI, a subagent, an eval, a batch), the loop **awaits the job inline**. Every
  path without a conversation host behaves exactly as today.
- `image_generate` splits at `backend.generate`: validation, claim and casting run before the
  split, and generate, save, near-copy and manifest run in the job. Its `now` is factual:
  `being made: images/<will-be>.png`. The path is reserved, so a later reference can name it.
- **The job is `'static`, so it owns what it uses.** `Tool::call` keeps its signature (`&self`,
  `&ToolCtx`); the tool builds the job from owned parts before returning:
  - an `Arc` of what `generate` and `save` need (the backend handle and the settings), never a
    borrow of the tool;
  - an **owned claim**: today's `Claim<'a>` borrows the tool and releases on `Drop` unless
    `keep()` ran, so it becomes a claim holding an `Arc` of the claim table. A job that is
    cancelled or fails drops it and releases the repeat guard at once, instead of leaving it
    stuck for `REPEAT_WINDOW`;
  - the output path, **resolved through `ToolCtx::resolve` before the split** and moved into the
    job as a resolved path, plus an owned clone of the context fields `save` reads. The job never
    resolves a model-supplied path, so the jail is proved in the call, as today.

### 2.2 The job queue (core): one per conversation

- `jobs.rs`: a `JobQueue` keyed by the conversation, holding at most **one** job in flight.
- A second deferred call while one is pending gets an immediate refusal, which is factual and
  structural, not advice. This bounds the in-turn runaway without any turn cap (R6).
  - **The words are the tool's, not the queue's.** A deferring tool says what its refusal reads
    (`ToolOutput::deferred(now, job).busy(text)`; `image_generate`'s is `not made: a picture is
    already being made`). The core queue stays generic and holds no picture prose.
  - **It is a refusal, not a failure:** the result carries `refusal: true`, as `refused()`'s
    results do, so a queue collision never books as a tool failure in the run-quality corpus.
    It is decided before the tool's job starts, so no GPU work is spent; for `image_generate`
    the refusal is raised through `refused()` itself, keeping that function the single exit for
    a picture that will not be made (review of #384, the 2026-09-28 incident).
- A job runs to completion, failure, or an explicit cancel (§2.4). Its outcome is held until the
  host can deliver it (§2.3).

### 2.3 Delivery: the call's result arrives late

When a job finishes, the host:
1. **Sends `WireEvent::ToolResult{name, id, is_error, preview}` at once**, with the call's
   original id and name. The page updates the existing tool entry, so the picture appears where
   the call is, in the chat and on the call screen. This happens even while another run is in
   flight, because the live entry is the page's. **A failed job sends `is_error: true`**, or the
   page's `turnsWithoutPicture` would read the turn as drawn.
2. **Rewrites the stored result** at the next point it holds the conversation: the end of the
   current run, or before the next one starts (the `pending_crisis` pattern).
   - "being made: …" becomes the finished, text-only result (`image: <path> …`), recorded as a
     `Record::Rewrite`.
   - No `Block::Image` is added: the model is told a picture reached the owner, never shown its
     pixels (`image_view` stays the separate step).
   - The rewrite touches a recent message, so the cached prefix is re-read from that point once.
3. **Adds a run note for the next run** (§5.1, `RunContext::notes` + `Record::Notes`): "the
   picture you started (images/x.png) has reached the owner; you have not seen it".

A failure takes the same path: the result becomes `not made: <reason>`.

**The rewrite finds its result by `tool_use_id`, never by position.** A compaction can cut the
message holding the "being made" result before the job finishes, the in-session counterpart of
the restart below. When no result with that id is left in the history, there is nothing to
rewrite: the page event (1) and the run note (3) still go, so the owner sees the picture and the
model is told; the summary's own wording, if it mentions the picture, stays what it was.

A server restart loses in-flight jobs. When a session is loaded, any `being made: …` result with no job behind it is
rewritten to `not made: the server restarted`, so no result claims a picture is still coming.

### 2.4 Talking never kills a job; Stop does

- **The job's token is not the run's.** A barge-in cancels the run, meaning the talking, and the
  job carries on.
- **The Stop button** (`/cancel` in both chats) cancels the run **and** the conversation's jobs:
  two calls at the call site, so no new `CancelReason` is needed.
- **Hang-up, leaving the chat, a model switch and a client disconnect** cancel the run only. A
  picture asked for is still delivered to the chat.
- **Server shutdown** cancels everything (`Shutdown`).

### 2.5 Both chats

The mechanism is core, and each chat's host supplies the sink and the delivery (its session
lookup, its `events` sender, its conversation ownership). The assistant chat and persona chats
get the same behaviour.

## 3. What this removes

- The 40 s silence on a call: the turn answers at once.
- The cancel-and-retry loop: speech never cancels a picture, so no "Cancelled" result tells the
  model its work is unfinished.
- The in-turn runaway: one job at a time, structurally.
- `image_generate`'s exception to "a tool is never interrupted mid-call": the job, not the call,
  watches its own token.
- **Not** the repeat guard's in-flight branch (`REPEAT_IN_FLIGHT`). The queue is per
  conversation and Q3 lets a second conversation's picture through, but the claim is keyed on
  `ctx.workspace`, which every persona chat shares (`work::producer_dir("persona")`). The
  in-flight branch is what collapses two chats asking for the identical render at once, so it
  stays (review of #573). The completed-repeat branch stays too, until §6 of the parent doc
  measures it.

## 4. What does not change

- `image_view` is still the only way the model sees a picture.
- Images reach the model only as user turns.
- The path jail is unchanged: the job writes through the same `save`, within the same workspace,
  to a path resolved in the call (§2.1).
- **Taint is armed by what came back, when it comes back.** Today the loop records a result's
  taint (`ToolOutput::external`) in the run that made the call. A late result arrives after that
  run, so the host arms the conversation's taint from the job's `ToolOutput` at delivery, by the
  same rule, before the next run starts, and records it with the rewrite. A job that reached
  outside and came back `external` therefore arms `untrusted` exactly as the inline call would
  have. `image_generate` talks to a loopback server and arms nothing today, but the mechanism is
  generic, and this is the rule that would otherwise be found missing later.
- The manifest, the near-copy measurement and the face anchor run as they do now, inside the job.

## 5. Tests (each fails on today's behaviour)

- A deferred call with a sink records `now` and returns before the job finishes. Without a sink,
  the result is the finished one (inline).
- A barge-in during a job: the run stops, the job completes, and the stored result becomes
  `image: …`.
- Stop during a job: the job is cancelled and the result becomes `not made: stopped by the owner`.
- A second deferred call while one is pending gets `not made: … already being made`, without
  touching the backend.
- Delivery while another run holds the conversation: the page event goes at once, and the
  rewrite and run note apply at that run's end.
- Loading a session with an orphaned `being made` result rewrites it to `not made: …`.
- The persona and assistant chats both deliver.
- Measurement (parent doc §8): picture latency and loops on a scripted call, before and after.

## 6. Build order

1. Core: `ToolOutput::deferred`, the `JobSink` and `JobQueue`, inline-await without a sink, and
   unit tests with a fake slow tool.
2. `image_generate` split. This lands after the parent doc's §5.2 and §5.5 have changed this file.
3. The persona chat host: sink, delivery (event, rewrite, run note), Stop vs barge-in, orphan
   repair on load.
4. The assistant chat host, the same way.
5. The scripted-call measurement.

## 7. Rulings (owner, 2026-10-05)

- **Q1. A finished picture is not announced.** It appears on screen, in the chat and on the call,
  and the persona learns of it on its next turn (§2.3). There is no unprompted extra turn.
- **Q2. On a call, a picture is stopped by a stop control on the call screen's picture slot**,
  shown while one is being made. It takes the Stop path of §2.4 for that conversation's job.
  Speech never stops a picture, and hanging up does not either.
- **Q3. One job per conversation.** ComfyUI still renders one job at a time, so a second chat's
  picture waits its turn in ComfyUI's queue rather than being refused.
