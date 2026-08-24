# Tasks: the board, and handing one to the agent

> **Status: designed, not built.** Written 2026-08-24 after the owner's first
> real day on the phone app. `TASK-RESEARCH.md` is the evidence; this is the
> argument. `REMOTE-SURFACE-DESIGN.md` §12 holds the rest of that day's
> backlog — this is the item large enough to need its own document.

Two asks, and they turn out to be one design:

> *"I don't like the horizontal filters on the top… I don't really get what
> this is. is it a way to organize tasks?"*
>
> *"I would like to be able to communicate with my agent about tasks so that
> it can do things for me. Right now, it is just a list of things, but I
> can't assign it to an agent or prompt an agent to do something to help
> complete a task."*

They are one design because the answer to both is **status**. The board's
views are status made legible, and handing work to the agent is a status the
board does not yet have a way to express.

---

## Part 1 — The board itself

### 1.1 What is there now, and why it did not read

`mecha tasks` and the phone's Tasks page are a GTD board over the knowledge
graph — capture, five statuses, due/defer/context, a project link — with
every verb going through the graph's MCP surface (`kg_task_list`,
`kg_task_create`, `kg_task_update`), which is what keeps mecha from growing
a second reader of somebody else's schema.

The page shipped with four chips reading `actionable · scheduled · waiting ·
done`, which are the *statuses* with no statement of what they mean. The
owner's *"I don't really get what this is"* is the correct response to a
filter that names its implementation. Fixed the same evening — the chips
became a left drawer with a sentence under each view — but the underlying
question is the one below.

### 1.2 What the research says mecha already got right

Two things, worth knowing before changing anything:

- **"Waiting For" is a first-class status here, and in none of the four apps
  surveyed.** Things, OmniFocus, Todoist and TickTick all make the user
  build it out of a tag or a saved query. mecha's board has it natively —
  and `waiting_on` is a *graph fact* (`gtd.rs`: subject = task, predicate =
  `waiting_on`, object = a person), so "who has the ball" is a queryable
  relationship rather than a string in a label.
- **Capture lands in Inbox and defers organization**, which is the one
  invariant every surveyed app shares. `kg_task_create` lands in `inbox` on
  purpose and says so in its own description.

The OmniFocus insight that matters most: **delegation is a status that
suppresses an item from "what can I do now" while keeping it reviewable.**
That is exactly what `waiting` + `waiting_on` is. Part 2 is, in one
sentence, *give that status a non-human object*.

### 1.3 Where the board is genuinely behind

| Gap | Evidence | Decision |
|---|---|---|
| One date field | Things splits **start date** (puts it in Today) from **deadline** (does not). mecha has `due_at` + `defer_until`, which is nearly the same split under worse names | Rename in the UI, not the schema: `defer_until` is "start", `due_at` is "deadline" |
| No review | **Only OmniFocus ships Review** as a perspective, with per-project intervals — the GTD weekly review, in the product | B4 below |
| No natural-language capture | Todoist/TickTick parse "call Bob tomorrow 3pm #work" inline; both treat it as table stakes | B2 below |
| Row actions are status nouns | Every app's invariant: **complete and schedule are one gesture; everything else opens a sheet** | B1 below |

### 1.4 Board decisions

**B1 — One-gesture complete, one-gesture schedule, a sheet for the rest.**
The row's action strip currently offers every status as a chip, which the
owner called *"a little bizarre"*, correctly: six equal-weight options where
the surveyed apps offer two. `✓ done` and `schedule` become the two direct
actions (schedule opening a mini date picker, not a form); everything else —
move to waiting, drop, change context, ask mecha — lives in a sheet behind a
`…`. This is the one place in the project where copying the consensus is
right: four apps that disagree about everything else agree about this.

**B2 — Natural-language capture, parsed deterministically, title kept
literal.** "Call Bob tomorrow at 3" should set a date. Two rules: the parse
happens in **Rust, not a model** (a capture that costs a model call is a
capture nobody uses, and a model that rewrites what you typed is worse than
one that does nothing), and it follows **Things rather than Todoist** on the
disagreement — the parsed token is *shown as a chip you can dismiss*, and
the task's name keeps the owner's words. A capture surface that silently
edits what you said is the wrong default for a store that is supposed to
hold your own intentions verbatim.

This pairs with dictation (shipped 2026-08-24): spoken capture and date
parsing are the same feature from the owner's side — *"remind me to call
Bob tomorrow"* said out loud.

**B3 — The views are named for what they mean, and there are few of them.**
Things ships six opinionated views and no user-defined ones; Todoist ships
three and a query language. mecha takes Things' side — the board has exactly
one user, and a query language is a feature for teams. The drawer's four
views stay, each with its sentence.

**B4 — Review is worth building and is not in this arc.** OmniFocus is alone
in shipping the weekly review, and it is the practice that keeps a GTD
system honest. Noted here so it is not re-derived from scratch; it wants its
own design, and it may want the agent (a review pass is exactly the kind of
bounded, repetitive reading a scheduled run is good at).

---

## Part 2 — Handing a task to the agent

### 2.1 The problem, stated precisely

The board is **a list of intentions in a system that can act**, and the two
halves never touch. The assistant can already read the board — the tools are
in its registry — but a person looking at *"Follow up with John about the
Psych 62 neuro approval"* has no gesture meaning *you do this part*.

The gap is not a missing model capability. It is **a missing noun**: a run
needs something to be *about*, and the task is that thing.

### 2.2 What already exists to build on

- **The seeded-run pattern**, `commands/mail.rs::draft`. An item becomes a
  fresh `Conversation`, a prompt built deterministically from the record, a
  recorded `Session` titled after the item, the outbox route bound to that
  session id, one interruptible run, and a **staged-id diff** naming exactly
  what the run produced. Mail's *"draft a reply to this thread"* is this
  document's feature, one noun over.
- **Sessions are resumable and visible** as of 2026-08-24 evening
  (`serve/chat.rs::resume`, the drawer). A run about a task can be a
  conversation the owner walks into later — which is literally what
  *"communicate with my agent about tasks"* asks for.
- **The outbox** catches everything outbound whoever drafted it, so a task
  run that writes an email needs no new review surface.
- **`ask_user` + `Questions`** (`serve/present.rs`) already deliver a
  mid-run question to a phone and take an answer back. Linear's
  `elicitation`/`awaitingInput` is the only shipped equivalent in the
  products surveyed; mecha has the machinery already and has never pointed
  it at a task.

### 2.3 The shape

**A task gains one verb: *ask mecha*.** It starts a run whose subject is the
task, in its own session, and moves the task to `waiting` with `waiting_on`
pointing at the agent.

```
task row ──ask mecha──▶ seeded run (own session, titled "task: <name>")
                          │
                          ├─▶ sends/writes ──▶ outbox (unchanged)
                          ├─▶ transcript ────▶ chat drawer (resumable)
                          └─▶ task ──────────▶ waiting · waiting_on mecha
```

Three properties fall out of the arrangement rather than being added to it:
the work is inspectable during and after (it is an ordinary recorded
session — `mecha sessions`, the drawer, `recall`); continuing the
conversation *is* resuming the session, not a second mechanism; and the
board tells the truth about who has the ball, because a task the agent is
working is neither `next` nor `done`.

### 2.4 Decisions

**D1 — The agent is delegated to, never assigned.** Linear rebuilt their
data model around this: issues "can only be *assigned* to humans, and only
*delegated* to agents", because **"an agent cannot be held accountable"** —
responsibility does not transfer. Their practical reason is as sharp: with
plain assignment "you'd sometimes see an agent with dozens of issues
assigned, but no clear sense of who was behind them."

mecha gets this nearly free, because `waiting_on` is already delegation
rather than ownership: the task stays the owner's, and `waiting_on` says who
it is currently blocked on. **No assignee field is added.** GitHub Copilot
took the other side ("assign it just like a human developer") and it is the
wrong side for a store whose entire purpose is one person's own
commitments.

**D2 — The run is a conversation from the start, not a fire-and-forget
job.** Mail's drafting is detached because a reply is a bounded artifact; a
task is not bounded. The cautionary number decides it: in ten months of the
Copilot coding agent on `dotnet/runtime`, **52.3% of agent PRs needed direct
human commits**, and success was **86.2% with intervention against 55.1%
fully autonomous**. A design that assumes the human joins the loop is
designing for the measured case; one that hopes they need not is designing
for the brochure.

So the run starts as a session the owner can open, read mid-flight, steer
(`RunContext::queued_input` exists), or answer a question in.

**D3 — A run gets more permission by acquiring a human, never by asking for
one.** An unattended start means the approver has nobody to ask, so the
first pass runs at the trigger posture: read-only, sends staged. Anything
needing more waits for the owner to open the session, where `ask` mode's
approval cards already work. This is the existing rule (`ModeApprover`'s
"nothing is watching to answer") pointed at a new surface, not a new policy.

**D4 — The seed prompt is built from the task, deterministically.** No model
writes it: the name, project, context, dates and the owner's optional note
become the prompt, exactly as `draft_prompt` does for a mail record. A
model-written seed is an unreviewed instruction entering a privileged run —
the front door's argument arriving through a different door.

The research pushes back usefully here: Copilot's success went from **38.1%
to 69% purely on tuning `copilot-instructions.md`**, and flowmail's
*Context Assembler* (deterministic, no LLM — card notes, memory, thread
context) exists for the same reason. **Setup dominates outcome.** The
project-neighbourhood context (`kg_related` on the project node) is
therefore likely worth its prefix cost — but it is measured in Phase 4, not
assumed in Phase 1.

**D5 — State is derived from the record, never self-reported.** Linear
derives an agent session's state from the last emitted activity; the agent
never sets it. mecha already holds this rule elsewhere — the TUI reads a
trigger's last answer from the session transcript rather than a cached copy,
because a second source of truth can disagree with the first. So: **the
task's status is moved by the harness at run start and run end, and the
model is never given a tool that sets it.** "Is work happening?" is answered
by the session, not by the agent's opinion of itself.

**D6 — The agent may not mark its own task done.** A run that can close its
own assignment is a lane promoting itself — `ladder.rs`'s oldest rule, one
store over, and the same reason `kg_accept` does not exist on the tool
surface. The run may add findings, stage sends, and report; the owner
disposes. Enforced by a narrowed tool surface on the run
(`Tool::narrows_surface_to`, which skills already use), not by a prompt
asking nicely.

**D7 — It must be stoppable, and stopping must be one gesture.** Copilot has
**no stop button** — a stuck session "will time out after an hour" and the
documented recovery is to unassign and reassign. mecha has cancellation
already (`RunContext::cancel`, which keeps the partial turn), so the task
row's *stop* is wiring, not invention. Absence of a stop button is a
documented failure in a shipping product; there is no excuse for
reproducing it.

**D8 — `waiting_on` the agent needs a graph change, and it is small.**
`kg_task_update` accepts `status`, `due`, `defer`, `context` — not
`waiting_on`, which is a fact written by the extractor. Preferred: **extend
`kg_task_update` with `waiting_on`** (a name that must resolve to an
existing node, like `project` does) and give the graph a `mecha` agent node.
Smallest change, keeps mecha's MCP-only rule intact, makes the existing
waiting view correct for free, and lets the board distinguish *waiting on
Nadia* from *waiting on the agent* — the distinction the owner will want to
filter on, and the one TickTick built a whole "Assigned to Me" smart list
for.

Rejected: a mecha-side store of "runs about tasks" — a second source of
truth about task state, disagreeing with the graph the first time a run
dies.

**D9 — The link from task to session is a fact, not a filename.** The run
must be findable from the task later. A session title (`task: <name>`) is a
convention, not an index, and titles are not unique. Preferred: a
`worked_on` fact written through the tool surface — the graph already holds
`originated_in` facts pointing at episodes, so this is a shape it knows.
The task row then offers *open the conversation*.

**D10 — Task sessions belong in the drawer, labelled.**
`serve/chat.rs::history` filters to titles starting `web: ` or `voice: `,
which would hide exactly these. The filter gains `task: `, and the row shows
a `task` chip beside `voice`. **A run the owner cannot find is a run they
will start twice.**

**D11 — One live run per task.** A second *ask mecha* on a task already
being worked resumes the existing session rather than starting a rival —
the rule the resume endpoint already enforces ("one conversation must never
have two writers"). `waiting_on` is the flag that says a run exists.

### 2.5 What this deliberately does not include

- **Recurring work on a task.** That is `[[trigger]]`, which exists and
  already refuses to live in project config.
- **Multi-task planning** (*"do my whole board"*). The run would be
  unbounded, the taint would be the union of everything it read, and the
  review surface would be a wall. Note that Asana's AI teammates decompose
  into subtasks as the visible plan — a good idea that needs the board to
  have subtasks, which it does not.
- **A second approval surface.** Sends stage in the outbox. Nothing here
  earns a new place for a human to click yes.

---

## Phases

**Phase 1 — the CLI verb.** `mecha tasks work <id> [--note ...]`: seeded run,
own session, outbox-bound, prints the session id and what it staged. Nothing
on the phone yet. *Verify: the run appears in `mecha sessions`; a send it
drafts appears in `mecha outbox`; the task's status moves; Ctrl-C stops it
and keeps the partial answer.*

**Phase 2 — the graph side.** `waiting_on` on `kg_task_update`, a `mecha`
node, the `worked_on` fact (D8, D9). *Verify: the waiting view distinguishes
agent-held from person-held tasks.*

**Phase 3 — the phone.** *Ask mecha* on the task row, *open the
conversation*, *stop*, the drawer filter and chip (D10). Board decisions B1
and B2 ride along, since they touch the same row. *Verify: tap → a session
appears in the drawer → opening it shows the work → steering it works → stop
works.*

**Phase 4 — narrowing and context.** The tool-surface narrowing (D6) and the
context assembler (D4), both of which the first three phases make
measurable.

## Open at design time

- **Does the first pass need a plan step?** *"Follow up with John"* is one
  action; *"prepare the Psych 62 materials"* is a project. Devin produces an
  "Interactive Planning" blueprint **with a confidence score** before
  acting, and Asana decomposes into subtasks. Whether mecha should is
  unanswered, and the honest way to find out is Phase 1 against real board
  items.
- **What does the owner see while it runs, on a sleeping phone?** The
  session streams over SSE to an open page and catches up on reload; push
  (remote-surface Phase 5) is the real answer and is not built.
- **Cost.** Every *ask mecha* is a full agent run on the local model, and
  there is no per-task accounting — `[agent] budget` is per run. Devin's
  most-cited complaint is exactly this opacity: with async delegation "you
  may not know the ACU tab until the invoice arrives." Local inference makes
  the money question moot and the **time** question sharper: the model is
  shared with chat, voice and triggers, so a task run is latency somebody
  else pays for. Worth measuring before the gesture is cheap enough to tap
  idly.
- **Task shape.** Copilot's success rate ran from 84.7% (cleanup) to 54.5%
  (performance work, which it cannot self-validate). The equivalent question
  here — which kinds of board item are worth delegating — can only be
  answered by running it, and `RunStats` already records enough to answer it
  later.
