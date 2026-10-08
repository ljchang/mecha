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
- `AgentEvent::ToolResult` becomes `WireEvent::ToolResult{name, id, is_error, preview}`.
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
  is a `DeferredJob` — a boxed future yielding the finished `ToolOutput`, its busy text, and
  `cancel`, **the job's own cancellation token, carried beside it** — held as an
  `Option<Arc<DeferredJob>>` field. The `Arc` keeps `ToolOutput`'s `Clone`; `Debug` takes a
  hand-written impl on `DeferredJob`, since a boxed future has none; and the future sits in a
  `Mutex<Option<…>>` that whoever runs it takes exactly once, because a future behind a
  shared `Arc` cannot be moved out to be awaited (review of #573, passes 9 and 10). The tool creates the token and builds the job to watch it,
  because a boxed future can be dropped but not handed a token afterwards, and the tool cannot
  know whether the run has a sink (review of #573, pass 4). Whoever runs the job holds `cancel`:
  the loop, linked to the run's token when it awaits inline; the host's queue otherwise.
- The agent loop hands the job to `RunContext::jobs` when the run has one (a `JobSink`, given by
  a chat host) and records `now` as the call's result.
- With no sink (the CLI, a subagent, an eval, a batch), the loop **awaits the job inline**. Every
  path without a conversation host behaves exactly as today, **cancellation included**: awaited
  inline, the run's cancellation is **linked** to the job's own token — a run's Stop cancels
  it — so a Stop or a Ctrl-C stops the render and `abandon`s the server's job exactly as the
  tool's own watch does now. Never by dropping the future: a dropped job leaves the render
  running on the server (review of #573, pass 11). Only a host's
  sink hands a job a token of its own (`subagent.rs`'s rule: a stop that left work running would
  be a lie). (Review of #573, pass 2.)
- `image_generate` splits at `backend.generate`: validation, claim and casting run before the
  split, and generate, save, near-copy and manifest run in the job. Its `now` is factual:
  `being made: images/<will-be>.png`. The path is reserved, so a later reference can name it.
  **Reserving it moves the stamp to the call**: today `save` stamps the name with
  `Utc::now()` when the bytes are written and chooses it itself, so the call takes the stamp
  (and the seed is final by then), names `images/<stamp>-<seed>.png`, and hands the stamp to
  `save`, which writes under it — or beside it, numbered, if something already sits there
  (review of #573, pass 16).
  `now` is `is_error: false`, and the page reads a `being made:` first line as **still out** —
  `picture.js` gains that third state beside drawn and not drawn — so `turnsWithoutPicture`
  never counts a picture in progress as drawn (review of #573, pass 6). The change lands in the
  counter itself, which reads an `image_generate` row as drawn on `is_error === false` alone;
  `pictureOf`'s `^image: ` pattern already rejects a `being made:` line (review of #573, pass 8).
  **The call screen draws the same third state** (`call-lines.js` `historyLines`): a
  still-out row holds its place in the call's transcript as "drawing a picture…", which the
  picture takes when it lands, and the stage holds the picture slot with its own Stop
  (ruling Q2). That Stop asks `/cancel` for the picture alone, so it never cuts off the reply
  the persona is speaking (review of #573, pass 9).
- **The job is `'static`, so it owns what it uses.** `Tool::call` keeps its signature (`&self`,
  `&ToolCtx`); the tool builds the job from owned parts before returning:
  - an `Arc` of what `generate` and `save` need (the backend handle and the settings), never a
    borrow of the tool;
  - an **owned claim**: today's `Claim<'a>` borrows the tool and releases on `Drop` unless
    `keep()` ran, so it becomes a claim holding an `Arc` of the claim table. A job that is
    cancelled or fails drops it and releases the repeat guard at once, instead of leaving it
    stuck for `REPEAT_WINDOW`. **And the table's keying changes with it**: one slot per
    workspace held both "last drawn" and "drawing now", so a second request claimed while the
    first was out overwrote its slot, and refused as busy, dropped its claim and erased the
    first. A workspace keeps the request that last drew *and a set* of those drawing now, each
    claim releasing only its own (review of #573, pass 15);
  - the output path, **resolved through `ToolCtx::resolve` before the split** and moved into the
    job as a resolved path, plus an owned clone of the context fields `save` reads. The job never
    resolves a model-supplied path, so the jail is proved in the call, as today.

### 2.2 The job queue (core): one per conversation

- `jobs.rs`: a `JobQueue` keyed by **the host's session key** (the chat's key, which both chats
  already use for their session maps — `agent::Conversation` has no identity of its own), holding
  at most **one** job in flight.
- A second deferred call while one is pending gets an immediate refusal, which is factual and
  structural, not advice. This bounds the in-turn runaway without any turn cap (R6).
  - **The words are the tool's, not the queue's.** A deferring tool says what its refusal reads
    (`DeferredJob::new(job, cancel, busy)`; `image_generate`'s is `not made: a picture is
    already being made`). The core queue stays generic, holds no picture prose, and builds the
    refusal from that text alone.
  - **It is a refusal, not a failure:** the core builds it with `ToolOutput::refusal`, so it
    carries `refusal: true` and a queue collision never books as a tool failure in the
    run-quality corpus. It is decided before the tool's job starts, so no GPU work is spent.
  - **`refused()` is not changed by this.** It builds `ToolOutput::err` today, so its 14 refusals
    book as tool failures. Whether they should be `refusal: true` is a separate decision, left
    open and outside this build; the queue's refusal does not depend on it.
- A job runs to completion, failure, or an explicit cancel (§2.4). Its outcome is held until the
  host can deliver it (§2.3). **A job whose run ended in error is cancelled at that run's
  hand-back**: the run was rolled back, call and all, so there is nothing left to show the
  picture to, and drawing on would hold the conversation's one slot — and, for a deferring
  tool with reach, its send gate — until the render ended (review of #573, pass 16).

### 2.3 Delivery: the call's result arrives late

When a job finishes, the host does the four steps below. They are listed by what they do, not
when: with the chat idle, the record (2) is written before the event (1) goes, which is the
ordering guarantee (1) states (review of #573, pass 10).
1. **Sends `WireEvent::ToolResult{name, id, is_error, preview}`**, with the call's original id
   and name. The page updates the existing tool entry, so the picture appears where the call is,
   in the chat and on the call screen, even while another run is in flight. **Both pages need
   a change for it**: the persona page's fold matches a row by id at any time, but the
   assistant page's `openCall` takes only a still-pending row, and the call answered at once, so
   its row is closed — it gains a branch that lands the result on a closed row whose result is
   "being made". And a row read back from the transcript must carry its call's id
   (`chat::transcript_entries`), or a page reloaded while the picture was drawn has nothing to
   match (reviews of #573 pass 14, #583 pass 4). **A failed job sends
   `is_error: true`**, or the page's `turnsWithoutPicture` would read the turn as drawn.
   **The live entry lasts only until the page next reads the transcript**: both chats rebuild
   their entries from the server's history on settle and on load, carrying over only page-only
   lines, which a tool row never is. So the event is a preview of the record, not a substitute
   for it: when the chat is idle the record (2) is written *before* the event goes, and when a
   run holds the conversation it is written at that run's end, before the run's own end event
   sends the page to re-read (review of #573, pass 8).
2. **Rewrites the stored result** at the next point it holds the conversation: **at once when no
   run holds it** (the chat is idle — the common case, since a picture outlasts the reply that
   started it), else at the end of the current run (the `pending_crisis` pattern). Never
   "before the next run starts": an idle chat may have no next run for hours, and a restart in
   that window would find a finished picture with nothing recorded about it (review of #573,
   pass 4).
   - "being made: …" becomes the finished, text-only result (`image: <path> …`), recorded as
     **an appended `Record::LateResult { index, tool_use_id, content, is_error, external }`** that
     `Session::read` applies to message `index`, the one holding that call's result.
     **`external` is the result's provenance**, written into that message's
     `tool_provenance` as the loop writes an inline one: without it a resume or a replay
     (`replay.rs` reads `tool_provenance`) would take an outside late result for one of ours —
     the content-only rewrite §4 rules out, surviving a restart (review of #573, pass 13).
     **`index` is what keeps it honest on taint**: `TaintTimeline::from_records` keeps no
     messages, so the record must say where it lands, and it takes `Record::Extend`'s *drop*
     — the checkpoints covering `index` and after go — but **unconditionally**, for any `index`
     the reader holds, in **both** timelines (`Session::parse` and
     `TaintTimeline::from_records`). `Extend` gates its drop on `index + 1 == messages`, which a
     mid-history late result never satisfies: borrowed whole, the rule would drop nothing, and an
     `external` late result would sit in a message `mecha learn` still classifies clean (review of
     #573, passes 7 and 12). Never a
     `Record::Rewrite`: `read` clears the taint checkpoints on a rewrite, which would cost every
     conversation that made a picture its provenance-gated learning (`Record::Extend` exists for
     the same reason). An older build skips the unknown record and shows "being made" (review of
     #573, pass 6).
   - **The late result is appended before the delivery's `Record::Taint`, never after.** Its
     rule drops every checkpoint covering `index` and after, which is every one a
     `Record::Taint` pushed for a message mid-history; written the other way round, the
     delivery's own checkpoint is the one dropped, `covering` reads unknown for the whole tail,
     and `classify_origin` takes that as untrusted — the provenance-gated learning a rewrite
     would have cost, by another route (review of #573, pass 11).
   - **`Session::read` checks the record before applying it**, the guard `Record::Extend` gets
     from its `index + 1 == messages` check: message `index` must be a user message holding a
     `ToolResult` with that `tool_use_id`. A record that does not match is skipped with a
     warning; its taint rule (checkpoints from `index` dropped) still applies, so a bad record
     fails toward unknown, never clean (review of #573, pass 8).
   - No `Block::Image` is added: the model is told a picture reached the owner, never shown its
     pixels (`image_view` stays the separate step).
   - The rewrite touches a recent message, so the cached prefix is re-read from that point once.
3. **Adds a run note for the next run** (§5.1, `RunContext::notes` + `Record::Notes`): "the
   picture you started (images/x.png) has reached the owner; you have not seen it". Written at
   delivery as its own record, `Record::PendingNote` — a new one, not `Record::Notes`, which is
   what the next run writes when it takes the note — and the file keeps it across
   a restart: never only re-derived from the rewritten result, which a compaction may have cut
   (review of #573, pass 5). **It retires at the next `Record::Outcome`**: the notes still owed
   are the `PendingNote`s after the last outcome (`Transcript::pending_notes`). Not at
   `Record::Notes`, which a run writes for any note it carries, owed or not. So a note taken by
   a run that then fails is owed again — on file, since the failed run wrote no outcome, and in
   the host's memory, which puts it back at the hand-back (review of #573, pass 16).
4. **Books a failed job as a tool error** of the run that made the call. The run closed before
   the failure arrived, so without it `doctor`'s tool-error threshold and the candidate gate's
   error rate would grow quieter as more work is deferred (review of #573, pass 5). Not a
   second `Record::Outcome`, which `runlog` would read as an extra run: a
   `Record::LateFailure { run, tool_use_id }`, where `run` is the ordinal of the run that made
   the call among the file's `Record::Outcome`s. **Bound when that run hands back, not when it
   submits**: the hosts write an outcome only for a run that ended `Ok`, so an ordinal reserved
   at submit would land a failed run's error on the next run. The sink carries the run's number
   in this process; the hand-back maps it to the outcome it wrote, counted on from the outcomes
   the loaded file already holds — a resumed chat's ordinals continue its file's, never restart
   at zero (review of #573, pass 12). A run that ended in error
   maps to nothing: it was rolled back, call and all, so its job's late result has no message
   to rewrite, no run to book against and no note to leave; its taint is still armed (review
   of #573, pass 9). The corpus reader, `Session::outcomes_attributed`, keeps no
   messages and no positions, only its rows in file order, so the ordinal is the one join it can
   make: one more arm adds a tool error to row `run`. A message index would have no reader there
   (`outcome_positions` is `Transcript`'s, and a summarising rewrite nulls it), and an id alone
   would need that reader to walk every message; the id stays for audit. The same increment
   lands in all three outcome readers — `Session::outcomes_attributed` (the corpus),
   `Session::outcomes`, and `Session::read`'s `Transcript::outcomes`, from which its `episode`
   fold is derived — through one shared function, or `sessions show` and the corpus would count
   the same run's errors differently (review of #573, pass 12).
   `doctor`'s trigger-ledger arm and `exp_report` read stores written at run end on paths with no
   chat host, so they are out of scope by construction (review of #573, passes 6–8).
   **A late failure resolves against the whole file's rows, not the rows read so far**: every
   reader collects them in its walk and applies them after it, so the ordinal books against
   its run wherever the record sits, and the order cannot misattribute it. (Pass 10 of this
   review said "always after its outcome, or it finds no row"; the build made the readers
   order-free instead, which is the sturdier of the two, and a test pins it — review of #573,
   pass 11.)

A failure takes the same path: the result becomes `not made: <reason>`.

**The rewrite finds its result by `tool_use_id`, and `index` only records where it landed.** A
compaction can cut the message holding the "being made" result before the job finishes, the
in-session counterpart of the restart below. When no result with that id is left in the history,
there is nothing to rewrite: the page event (1) and the run note (3) still go, so the model is
told and the owner sees the picture, **until the page next re-reads the transcript**, which no
longer holds the call (1). The picture itself stays in `images/` and the gallery; that the chat
line does not survive a re-read on this path is a known limit, not a promise (review of #573,
pass 8). The summary's own wording, if it mentions the picture, stays what it was.

A server restart loses in-flight jobs. When a session is loaded, any `being made: <path>` result
with no job behind it is repaired by **asking the artifact, not the absent job**: if a manifest
in `images/` names the call — at the path §2.1 reserved, or at the numbered name beside it that
`save` takes when something already sits there — the job finished before the restart and the
result becomes the finished one (`image: <path>`, without the measurements the job did not get to record); if it
does not, the result becomes `not made: the server restarted`. Either way no result claims a
picture is still coming, and none denies one the owner already saw.
**The repair does delivery's step 2 alone, deliberately.** No run note (step 3): the repaired
result's own words say what happened ("finished before the server restarted"), and it sits in
the history the next run reads whole; a note that a live delivery leaves exists because the
result changed under a conversation already in flight, which a resumed one is not. No late
failure (step 4): the run that made the call is not known from the file — its outcome, if any,
does not name its calls — and a guess would book against the wrong run, which step 4 exists to
prevent. And the page event (step 1) has no page to go to: the chat is being opened (review of
#573, pass 17).
- **The manifest gains the call's `tool_use_id`** — a change, not today's shape: the manifest
  JSON has no such key, and `write_manifest` runs on `run`'s path after the near-copy check,
  not inside `save`. `ToolCtx::call_id` is stamped on every call, so the value is there.
- **A picture at the path counts only if it is the job's own.** The reserved name is told to
  the model 40 s before the bytes exist, and a run in the same jail can write that path
  (`fs_write`, `shell`), so a file there proves nothing alone. The job's manifest
  (`write_manifest`, written on the job's path after `save` and the near-copy check) names the call's
  `tool_use_id`, and the repair accepts the picture only when the manifest beside it does;
  anything else is `not made: the server restarted` (review of #573, pass 6).
- **The manifest lands after the bytes**, so a restart between the two leaves a picture with no
  manifest. That case reads `not made` and is honest: the job sends nothing — no page event, no
  record — until its manifest is written, so a picture without one is a picture nobody was
  shown. The file stays in `images/` as an orphan, harmless and visible in the gallery (review
  of #573, pass 8).
- **The repair fails closed on taint and provenance, by branch.** It has no job `ToolOutput`
  to read. On the picture branch the result *is* the tool's output, so the tool's declared
  capability stands in for its provenance: a tool that can return untrusted content is
  recorded `external`, `untrusted` armed, and a `Record::Taint` written — a mark on the
  conversation, which is not what `turn_taint` does (that one only ever blocks a send, and is
  not cited here). On the `not made: the server restarted` branch nothing came from outside:
  the words are the harness's, recorded not external, which is `ToolOutput::external`'s own
  rule. `image_generate` declares no outside reach, so both of its branches record not
  external. Never a content-only rewrite (review of #573, passes 5 and 10).
- **`Session::messages_ever` does not read the late records**, deliberately: its two readers
  that key on a result's id, `outbox_source` and grounding, are first-seen-wins, so a late
  result admitted there would be shadowed by the "being made" result it replaces, which
  already names the reserved path. Admitting one is a decision for the first deferring tool
  whose late content a claim could cite (review of #573, pass 10).

### 2.4 Talking never kills a job; Stop does

- **The job's token is not the run's.** A barge-in cancels the run, meaning the talking, and the
  job carries on.
- **`/cancel` has three meanings, and only these.** A barge-in cancels the run only. The Stop
  button cancels the run **and** the conversation's job (at most one, §2.2): two calls at the
  call site, so no new `CancelReason` is needed. The call screen's picture slot (ruling Q2)
  sends `{"picture": true}` and cancels the job alone, so the reply being spoken goes on
  (review of #573, pass 10).
- **Hang-up, leaving the chat, a model switch and a client disconnect** cancel the run only. A
  picture asked for is still delivered to the chat.
- **Server shutdown** cancels the runs (`Shutdown`) and drops the job with the process, without
  abandoning the image server's render; the next resume settles its result from the
  workspace (§2.3, the restart repair).

### 2.5 Both chats

The mechanism is core, and each chat's host supplies the sink and the delivery (its session
lookup, its `events` sender, its conversation ownership). **Three kinds of conversation, two
behaviours.** A kept assistant chat and a persona chat defer, the same way. An incognito chat
does not: it has no `Session`, so neither the record half of delivery nor the restart repair
exists there, and its run gets no sink — its picture is drawn inline, as before this design
(review of #573, pass 11).

**A conversation with a job out is not let go of.** The assistant host removes a chat from its
map on archive, delete and a task hand-over; each refuses while the chat's job runs, as each
refuses while a run does, because the result lands in this process's copy and a reopened one
would settle it as never made. Run numbers are process-wide, so a reopened chat's runs never
meet an earlier incarnation's job (review of #583).

**A `post_tool` hook sees the immediate answer only** ("being made: …"); the late result
reaches no hook (review of #583).

## 3. What this removes

- The 40 s silence on a call: the turn answers at once.
- The cancel-and-retry loop: speech never cancels a picture, so no "Cancelled" result tells the
  model its work is unfinished.
- The in-turn runaway: one job at a time, structurally.
- `image_generate`'s exception to "a tool is never interrupted mid-call": the job, not the call,
  watches its own token.
- **Not** the repeat guard's in-flight branch (`REPEAT_IN_FLIGHT`, retired 2026-10-08 with the
  repeat guard; one picture per run replaces it, `IMAGE-DESIGN.md` §5.5), though its wording changes:
  it says the colliding call is "in this turn" and offers that call's result, and under jobs the
  collision is another conversation's, whose result this run cannot use. The queue is per
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
- **A sender is never deferred.** The gate below covers the sends *after* a deferral; it
  cannot cover the deferred job's own send, which the interlock cleared against the taint of
  the turn that made the call. So the core hands a job to a sink only when the tool's declared
  egress is not `Chosen` (and its reach is known); a `Chosen` tool's job is awaited inline,
  inside the turn that cleared it. `image_generate` is `Egress::None` and defers; the rule is
  for the next tool (review of #573, pass 15).
- **The in-flight window is gated, not tainted.** Until a job from an `untrusted_input` tool is
  **delivered** — not merely finished: a job that ends mid-run waits for that run's hand-back,
  so the queue keeps it pending until the host says it landed (review of #573, pass 9) —
  every run of that conversation folds the tool's declared capability into its
  per-turn gate — the `turn_taint` mechanism, extended by the conversation's pending jobs — so
  an `Egress::Chosen` call between the deferral and the delivery is refused as it would be
  after the result had arrived. It does **not** arm `convo.taint`: that is permanent, and would
  cost the session its learning for a result that may come back clean. Delivery then arms
  `convo.taint` from the real provenance (review of #573, passes 6 and 7).
- **Taint is armed by what came back, when it comes back.** Today the loop records a result's
  taint (`ToolOutput::external`) in the run that made the call. A late result arrives after that
  run, so the host arms the conversation's taint from the job's `ToolOutput` at delivery, by the
  same rule, **unconditionally**, and persists it with its own `Record::Taint`: never only with
  the rewrite, which the compaction path of §2.3 skips, and never through the run note, which
  arms nothing (a plain text note matches no stem in `Taint::arm_for_content`). (Review of #573,
  pass 2.) A job that reached
  outside and came back `external` therefore arms `untrusted` exactly as the inline call would
  have. `image_generate` talks to a loopback server and arms nothing today, but the mechanism is
  generic, and this is the rule that would otherwise be found missing later.
- **And taint is one of five things the loop does to a result; delivery does four, and drops
  the fifth.** After a call executes, `run_tools` records its provenance
  (`Message::tool_provenance`, read by `replay.rs`), caps it to the turn's byte budget
  (`cap_result`), wraps an `external` result from an `untrusted_input` tool in the
  untrusted-content envelope, and arms taint — and the fifth: a picture the tool returned
  (`ToolOutput::image`) goes into the turn as an image block, arming `private_data` from its
  pixels. A late result carries no image: delivery drops it, so the late picture reaches the
  model only as its path, and `image_view` stays the one way it is seen, as §4 already says.
  Nothing to arm, then, because nothing entered (review of #573, pass 17). A late
  result goes through the same steps, by one function factored out of `run_tools` and called
  from both, with the cap the call's own turn had. A rewrite that changed only the content
  would replay an `external` late result as not-external (review of #573, pass 4).
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
- Loading a session with an orphaned `being made: <path>` result: with a picture at the path,
  the result becomes `image: <path>`; with none, `not made: the server restarted`.
- A job that finishes while the chat is idle rewrites its result at once, so a restart right
  after leaves the record already true.
- A late `external` result from an `untrusted_input` tool is stored with its provenance, capped,
  and wrapped in the envelope, exactly as the same result inline.
- The persona and assistant chats both deliver.
- Measurement (parent doc §8): picture latency and loops on a scripted call, before and after.
- A late result that came back `external` arms `untrusted` on the conversation, and records it,
  even when a compaction removed the result before delivery (no rewrite happens on that path).
- Without a sink, Stop during an inline-awaited job cancels it: the render stops and the server's
  job is abandoned, as today.
- A queue collision is a `refusal: true` result with the tool's own busy text, and books as no
  tool failure.
- A late result is an appended `Record::LateResult`: checkpoints before the patched message
  stand, every checkpoint at or after it drops, and `covering` that message reports the
  delivery's taint (or none), never clean — asserted in both `Session::parse`'s timeline and
  `TaintTimeline::from_records`, for a message that is not the last; an older build reads the
  file with the call still "being made".
- A `Record::LateResult` whose `index` does not hold a result with its `tool_use_id` is skipped,
  and the message's taint still reads unknown, never clean.
- After delivery of a *clean* late result, the conversation is not untrusted and its reflections
  still classify `Clean`: the wait gated sends per turn and armed nothing.
- An `Egress::Chosen` call between a deferred `untrusted_input` call and its delivery is
  refused.
- A deferring tool whose egress is `Chosen` is awaited inline even with a sink: the queue never
  sees it.
- Two requests claimed in one workspace at once: the second dropped leaves the first claimed,
  and the first's `keep` is the one a repeat is refused against.
- Restart repair with a picture at the reserved path whose manifest names another call (or
  none): `not made: the server restarted`; with the job's own manifest at the numbered name
  `save` took instead, the picture (review of #573, pass 14).
- A page that read the transcript while the picture was out gets it when it lands: the
  re-read row names its call.
- A late failure adds one tool error to the run that made the call, and no run to the corpus;
  `sessions show` and the corpus report the same count for that run.
- A late failure of a run that ended in error books nothing — not even on the next run, which
  an ordinal bound at submit would have charged — and its owed note is owed again.
- A late failure books by its ordinal wherever it sits in the file (the readers apply them
  after their walk).
- A job whose run ended in error is cancelled at the hand-back.
- A late result whose output carried an image lands as text only: no image block enters the
  conversation, and nothing is armed from pixels.
- The restart repair writes the late result and its taint and nothing else: no note, no late
  failure.
- An incognito chat's run gets no sink: its picture is drawn inline.
- A live picture entry survives the page's next re-read of the transcript: the record was written
  before the event.
- The page draws a `being made:` result as still out, never as drawn.


## 6. Build order

1. Core: `ToolOutput::deferred`, the `JobSink` and `JobQueue`, inline-await without a sink, and
   unit tests with a fake slow tool. The three new records trip `runlog::exhaustive`, which
   names every `Record` variant so the compiler forces a decision: `LateResult` and
   `PendingNote` are ignored by the corpus scan, and `LateFailure` is not — the scan reads it
   through `Session::outcomes_attributed` (review of #573, pass 13).
2. `image_generate` split. This lands after the parent doc's §5.2 and §5.5 have changed this file.
3. The persona chat host: sink, delivery (event, rewrite, run note), Stop vs barge-in, orphan
   repair on load; and the page: the still-out state in the chat and on the call screen
   (`picture.js`, `call-lines.js`), with the call screen's picture-only Stop (review of #573,
   pass 9).
4. The assistant chat host, the same way — and its page's `openCall`, which takes only a
   pending row, gains the closed "being made" row as a place a late result lands (review of
   #573, pass 14).
5. The scripted-call measurement.

Incognito needs no step of its own: it is the absence of one. Its run is simply not handed a sink
(step 4's host checks for a kept session), and §5 has a line for it.

## 7. Rulings (owner, 2026-10-05)

- **Q1. A finished picture is not announced.** It appears on screen, in the chat and on the call,
  and the persona learns of it on its next turn (§2.3). There is no unprompted extra turn.
- **Q2. On a call, a picture is stopped by a stop control on the call screen's picture slot**,
  shown while one is being made. It takes the Stop path of §2.4 for that conversation's job.
  Speech never stops a picture, and hanging up does not either.
- **Q3. One job per conversation.** ComfyUI still renders one job at a time, so a second chat's
  picture waits its turn in ComfyUI's queue rather than being refused.
