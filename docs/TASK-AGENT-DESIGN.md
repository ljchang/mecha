# Tasks: the board, and handing one to the agent

> **Status: phases 1–3 built 2026-08-26; 4–6 open.** Written 2026-08-24 after the owner's first
> real day on the phone app. `TASK-RESEARCH.md` is the evidence; this is the
> argument. `REMOTE-SURFACE-DESIGN.md` §12 holds the rest of that day's
> backlog — this is the item large enough to need its own document.
>
> **Extended 2026-08-26** with D12–D16 and Part 3, after a design pass on what
> a task-scoped chat surface actually needs. The open plan-step question the
> first draft left is resolved by D12; Part 3 records a refusal, because the
> KV-offload idea below has now been proposed twice.

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

> **Amended 2026-08-26, after phase 4 shipped.** The decision above holds on
> its evidence and is wrong in its translation of it. Both halves below;
> build the amended one.
>
> **What survives.** *ask mecha* belongs behind the extra tap, and not merely
> because this was written first — §1.3's invariant names it: *"complete and
> schedule are one gesture; move, **delegate** and edit open a sheet."*
> Delegation is in the sheet list explicitly. A later reading that promotes it
> because it is this project's differentiator is re-deciding a settled
> question on taste.
>
> **What does not.** §1.3 surveys **swipe** actions on a *collapsed* row, and
> mecha's row has no actions at all until it is tapped open. So the six chips
> the owner called *"a little bizarre"* were never in anyone's way — they
> appear only after a deliberate act, and **the expanded card already is the
> sheet.** Putting a `…` inside it nests a sheet in a sheet, which is worse
> than the thing it fixes. The strip's problem is not depth, it is that six
> equal-weight chips have no shape.
>
> Note also what B1 dropped when it turned "one gesture" into "one button":
> in Things, complete is *"a tap on the circle only"* — not a swipe, and not
> a button in a strip. mecha's rows have no circle, so the single most
> frequent action on a board is the one action that costs an expand.
>
> **So:** `✓` becomes a tap-target on the **collapsed** row. The expanded card
> **groups** rather than hides — lifecycle, delegation, provenance — and
> `schedule` keeps its mini date picker per §1.3. Nothing moves behind a
> second tap, because the first tap already bought one.

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

> **Amended 2026-08-26.** The decision holds; its worked example does not, and
> one thing it left unsaid is the real win.
>
> **The example is unrepresentable.** *"Call Bob tomorrow at 3"* cannot set a
> time, because there is nowhere to put one: `gtd::parse_due` accepts exactly
> `today | tomorrow | +Nd | YYYY-MM-DD`, and `due_at` is written
> `%Y-%m-%d`. **The board has no time-of-day.** So the chip reads `tomorrow`
> and *"at 3"* stays in the name — which is, at least, consistent with the
> Things side of the disagreement this decision already picked. Adding a time
> is a schema change in the other repo and is not in this arc.
>
> **The unsaid win: capture collapses to one box.** The sheet currently asks
> for a name, a due date and a context, which quietly breaks §2.1's own
> invariant — *"capture lands in Inbox and organization is deferred; no app
> makes you choose a project at capture time."* One box that parses is not a
> convenience on top of three fields, it is the removal of two of them.
>
> **And it is worth more than when it was written**, because dictation landed
> in between. A spoken capture arrives as **one string** with no second field
> to fill, so without this the microphone can only ever produce an undated
> inbox item. B2 is what makes that button worth pressing.
>
> **One split to hold:** mecha detects the *token* and hands it to the
> graph's `parse_due`; it does not resolve dates itself. Two date parsers in
> two repositories is the divergence this project refuses everywhere else,
> and the graph already owns the meaning of `+3d`.

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

> **Amended 2026-08-26, when Phase 5 was built.** The decision holds — the
> seed is still built by code from the record, and no model writes it. What
> changes is the *assembler*: it points, and does not paste.
>
> **Three things were already true that this decision predates.** The run's
> tool surface holds `kg_search`, `kg_entity`, `kg_related` and
> `kg_timeline`, so the neighbourhood is one call away rather than something
> only the seed could deliver. `kg_task_list` returns `captured_from` on
> every row — the provenance pointer that shipped the same day — so a task
> captured from an email knows which thread asked for it. And a seed is the
> front of a cached prefix that every turn of every task run re-sends, so
> pasted context is paid for on all of them while a sentence naming a tool
> is paid once and followed only by the runs that need it. That is
> `skill.rs`'s progressive disclosure, one door over.
>
> **And a constraint this decision does not state, which decides the shape.**
> `captured_from` can point at *mail*. Pasting a thread body into the seed
> would arm `untrusted` before the run's first turn **and** put
> attacker-controlled bytes into a privileged run's opening instruction —
> `frontdoor::Record::for_privileged_run`'s argument arriving through a third
> door. So the seed carries the **pointer**, never the content: kind, id,
> account and timestamp, and never the `label`, which is a subject line and
> therefore prose somebody else composed. The bytes arrive as a tool result,
> where the interlock accounts for them and the `<untrusted-content>`
> envelope is already around them. Any future assembler inherits that rule.
>
> **What the seed gained** (`Reach`, `commands/tasks.rs`): the provenance
> line, `defer_until` (on every row and previously dropped), a bullet naming
> the mail reader when the capture is mail *and* this surface holds one, and
> a bullet naming the graph lookups it holds. Tools are named by their
> **registered** name — `prefix_tools` makes `mail_get_thread` into
> `mail__mail_get_thread`, and a seed naming a tool the run cannot dispatch
> is the level-3 skill bug, which was found by running it rather than by
> reading it. A capture kind with no reader (`frontdoor`, `session`) is named
> as provenance and offered nothing.
>
> **The measurement this decision asked for is now the next step rather than
> a premise.** *"Measured in Phase 4, not assumed in Phase 1"* was never
> honoured, and can be from here: task runs write a `Record::Outcome` as of
> 2026-08-26, and whether a run follows the pointer is a scan of task-titled
> transcripts for a call to the tool the seed named. Paste only if they do
> not.

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

**D9 — The link from task to session is an index, not a filename — and an
attribute, not a fact.** The run must be findable from the task later. A
session title (`task: <name>`) is a convention rather than an index, and
titles are not unique.

The first draft proposed a `worked_on` fact, reasoning that the graph
already holds `originated_in` facts pointing at episodes. **Reversed on
2026-08-26 while building it**, for a reason that only shows up in
`distill.rs`: an episode is *evidence of what happened*, so it exists only
after a run **and** only when the distiller judged the run worth
remembering — and it deliberately does not for "smoke tests, one-line
lookups, greetings, aborted or purely mechanical runs", marking those
skipped forever. Those are exactly the runs a person half-remembers and
wants to reopen. An edge-based link would therefore be missing precisely
where it is wanted and present only where a summary already says what
happened.

Creating the episode at run *start* to close that gap was considered and
refused: it inverts the same rule from the other side — evidence of
something that has not happened — and pre-empts a judgement the distiller
exists to make, into a review queue already holding five figures.

So the session id is a **task attribute** (`kg_task_update`'s `session`,
stored on the node's properties, shown in the board row's tail). It is also
the more general of the two, which settles it: `distill::upsert_args` sends
`source_id: session_id`, so the episode's idempotence key *is* the session
id and a task holding it finds the episode too, whenever one appears. The
`originated_in` edge stays available later as an **addition** — it answers
traversal and provenance where this answers "which conversation", and two
answers to different questions cannot disagree.

**D9a — The agent is a node kind, not a person.** `waiting_on` was seeded
as "Task is waiting on Person" and the shortcut is to file `mecha` as one.
That is wrong for D1's reason: an agent cannot be held accountable, so
delegation is not assignment. A person node would also put the agent in
every people-shaped view — who owes me things, who I collaborate with — and
answer "who is responsible" with the wrong kind of thing. `agent` joined the
graph's closed node-type set, `agent-mecha` ships with the schema, and the
predicate's description says Person **or** Agent, because that description
is what a reader and the extractor go by.

**D9b — `@owner` names whoever the graph is about.** The callers handing
work back to a person are harnesses, and a person's name is exactly what a
harness should not carry — it would ship in config on every machine and be
wrong the day it changes. The graph already records its owner explicitly
(`owner_node`), so `waiting_on: "@owner"` asks it. A graph with no owner set
is a named failure, never a silent no-op: "waiting on nobody" and "waiting
on you" are opposite states and the board is the one place that must not
confuse them.

**D10 — Task sessions belong in the drawer, labelled.**
`serve/chat.rs::history` filters to titles starting `web: ` or `voice: `,
which would hide exactly these. The filter gains `task: `, and the row shows
a `task` chip beside `voice`. **A run the owner cannot find is a run they
will start twice.**

**D11 — One live run per task.** A second *ask mecha* on a task already
being worked resumes the existing session rather than starting a rival —
the rule the resume endpoint already enforces ("one conversation must never
have two writers"). `waiting_on` is the flag that says a run exists.

**D12 — The plan is a living list, and the gate is on its first version
only.**

> **Superseded 2026-08-26. Not built, and not to be built from this section.**
> The reasoning below is kept because it is where the argument was made; what
> replaced it, and why, is at the end of this decision. The short version: the
> gate made the *todo list* the human-editable object, and every other system
> — superpowers, Claude Code's plan mode, Cline's Plan/Act — keeps the
> reviewable plan and the agent's execution ledger apart. `todo.rs` already
> forbade the collapse and nobody noticed the conflict was that one.

The first draft left "does the first pass need a plan step?" open. It
does — but the answer has to be stated carefully, because `todo.rs` already
argues the other way in its own module doc:

> *Planning as a tool rather than a mode. The alternative — a "plan phase"
> that produces a plan and then hands off — goes stale the moment the first
> step surprises the model.*

That argument stands and is not overturned here. What it refutes is a
**frozen** plan, and alignment does not need one. A delegated run pauses after
its first `todo` write, shows the list, and takes the owner's edits; from then
on the list is rewritten on successes, failures and new thinking exactly as it
is in a chat run. The gate is on version one, not on the plan.

The evidence for gating at all is the setup finding: Copilot's success moved
from **38.1% to 69% purely on tuning `copilot-instructions.md`**, and the
intervention numbers (86.2% with, 55.1% without) say a human joins the loop
regardless. Questions asked at plan time are the cheapest in the run — before
a tool has been called, before any taint is armed, while a correction still
costs nothing.

Two rules on the gate: it fires **only for delegated runs**, since a chat turn
that happens to write a list is not a delegation and must not stop; and each
open question **carries a proposed answer**. The measured `ask_user` finding is
that telling a model to proceed with its best interpretation makes it invent —
a visible default a person taps is the opposite arrangement, because the guess
is on the screen and a human owns it.

**What replaced it.** Three findings, none of which was available when this
was written:

1. **`docs/VERIFICATION-RESEARCH.md` argues against plan-first on this
   hardware.** FORGE 2026 (48,000 scenarios, 6 models, the only large study
   with a non-agentic baseline) finds straight-shot often equals or beats
   both ReAct and Plan-and-Execute; **small models collapse** under
   plan-and-execute — Llama 3.2 3B goes 0.23 → 0.05; and **a bad plan
   measures worse than no plan**. mecha's entire premise is a local
   open-weight model. That doc existed and this section did not cite it.
2. **The trigger rested on a behaviour measured absent.** "After its first
   `todo` write" assumes the write happens. On 2026-08-04 this model called
   `todo` **zero times in 20 eval case-runs** whether the directive sat in
   the system prompt, the tool description, or both — and keeps a list
   reliably only when the *user turn* asks. So the gate would have fired
   when the model felt like letting it, which is a strange property for an
   alignment checkpoint.
3. **The evidence cited above is about the seed, not the gate.** Copilot's
   38.1% → 69% came *"purely on tuning `copilot-instructions.md`"* — the
   equivalent here is `work_prompt`, not a checkpoint. The 86.2%/55.1%
   intervention split argues for a human in the loop, which D13's question
   store already is.

So the intervention went into the seed: **work out what you need and ask it
first, in one question**, with a guard against the opposite failure (a run
that asks about what it could have looked up), delivered on the user turn
because that is the channel the probe found this model obeys. Questions at
plan time are still the cheapest in the run — before a tool call, before
taint is armed — which was D12's best argument and survives without it.

**What this does not reach, and how the decision gets remade.**
Front-loading improves *known* unknowns; a confidently wrong plan asks
nothing. That failure is now countable — delegations that ended `ready for
review` and were then dropped or reworked rather than marked done — because
`RunStats` on task runs and the question store's timestamps both exist as of
2026-08-26. If that number is large, build the reviewable object then, and
build it as a **document separate from the todo list**, which is the split
this section got wrong. The asymmetry that would force it sooner: a
web-launched delegation is `--unattended`, so it can only read and stage,
and letting a run go is bounded by construction — which stops being true the
moment a delegated run can acquire a present human's approval.

**D13 — A question ends the run; it never parks it.** `serve/present.rs` parks
the run on `ask_user` for `ASK_TIMEOUT` (600s) and then declines. That is right
for a *present* human — a page open in a hand — and wrong for a task, where the
honest case is that the owner answers at breakfast.

So a delegated run needing an answer **finishes**: the partial work is kept, the
question is stored, and `waiting_on` moves from `mecha` to the owner. Answering
resumes the session with the answer as the next user turn and moves `waiting_on`
back.

Three things fall out of the arrangement rather than being added to it. The
ball-passing is **already modelled** — `waiting_on` alternating between owner
and agent is the GTD semantics the board has natively, so the Waiting view
becomes the queue of blocked delegations with no new noun (§1.2). **No slot is
held**: a parked run occupies one of four (Part 3) and a cached prefix for ten
minutes doing nothing. And **`/queues` gains its sixth row** — an unanswered
question is precisely the kind of store that reaches 6,434 items without anybody
deciding to let it.

This is the outbox's argument in the other direction. The outbox exists because
a run's *outbound* action had to survive the run's end — staged by one process,
released by another, hours later. Nothing let a run's *question* do the same.
It is not a second approval surface (§2.5): nothing is approved here.

**D14 — The todo list is keyed per run, never per agent.** `TodoTool` holds
`Mutex<Vec<TodoItem>>` "for the lifetime of the agent", which was correct while
every front-end holding one had a single conversation. `mecha serve` is **one
shared agent** across every session, so two delegated runs share one list and
overwrite each other — and the card renders the wrong task's plan.

The precedent is in the same building: `present.rs` faced one agent and many
sessions' questions, and keys on the run's jail, whose file name *is* the
session key (`Asker::ask_in`). The todo tool takes the same treatment. Required
**before** parallel task runs exist rather than after — two concurrent tasks are
the first thing to exercise it, and the symptom is a plausible-looking list
belonging to something else.

**D15 — The list rehydrates from the transcript, never from a second store.**
The list lives in memory on the tool, which is fine for a run that ends when its
conversation does. A task outlives its run by construction (D13), and on resume
the model re-reads its plan from the last `todo` result in the transcript while
`TodoTool::items()` — what the card renders — comes back empty. The model knows
the plan and the UI shows no progress: D5's divergence, arriving from the other
side.

So the list is reconstructed from the last `todo` result on resume, and
deliberately not persisted beside the session. A second copy is the thing that
can disagree with the record — the objection that killed a mecha-side store of
task runs (D8), and the reason the TUI reads a trigger's last answer from the
transcript rather than a cached copy.

**D16 — The card's state is derived, and no two states render alike.** D5 says
status is moved by the harness and never self-reported; the card is where that
becomes visible, so the state set is named here: `idle`, `planning`, `working`
(with the current `[~]` item as the subtitle), `waiting on you`, `failed`,
`ready for review`.

Two carry the weight. **`waiting on you` must be loud** — it is the only state
that stalls indefinitely and the only one whose remedy is a person. And
**`failed` must never render as `idle`**: doctor's dash-never-zero rule, one
surface over, because "nothing is happening" and "it broke" are opposite
findings, and a card that renders them alike is how a delegation that died looks
like one nobody started. `ready for review` is the agent proposing completion
with its evidence attached — what it staged, what it read, its `RunStats`. It is
not `done`; D6 stands.

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

## Part 3 — Resources: what is scarce, and what is deliberately not built

### 3.1 The physical budget

The local server runs `-np 4` against `CTX 1048576` — four slots of 262,144,
which is the number `context_window` must equal and the arithmetic
`scripts/start-moe-mtp.sh` exists to keep honest. So **four runs can be in
flight at once**, shared across chat, voice, triggers and delegated tasks, and
a fifth waits.

KV costs **22 KiB per token** here: the model is hybrid attention, 11 of 41
layers holding a cache and 30 carrying a constant-size recurrent state, which
is why the figure is 22 and not the 82 a per-layer count predicts. A full
262,144-token slot is therefore ~5.5 GB, matching what the server reports.

`-cram 32768` is the prompt cache — 32 GB holding **evicted slot states, so a
returning prefix is restored instead of re-prefilled**. Raised from the 8 GB
default after 341 evictions in a day; none since. What it buys:

| A parked session of | is | fits in 32 GB |
|---|---|---|
| 30,000 tokens (a typical task) | 0.63 GB | **~50** |
| 60,000 tokens | 1.26 GB | ~25 |
| 262,144 tokens (a full slot) | 5.5 GB | ~6 |

Roughly fifty parked task conversations, today, with nothing new built.

### 3.2 R1 — Admission control, not memory management

The scarce resource is **a slot, not memory**. `-c` is divided across slots and
allocated at startup, so all four slots' KV is committed whether or not anything
occupies them; an idle conversation costs no extra VRAM. What contends is four
scheduling seats.

What to build in mecha is therefore a permit count, with one rule that should be
decided now because the alternative is discovering it by having it happen: **an
interactive turn preempts a background task run.** The owner typing must never
queue behind three delegations. `batch.rs`'s bounded-concurrency fan-out is the
shape to copy.

Priority beyond that **derives from the board and is not a field anybody
maintains**. `due_at` and `defer_until` are already there; a separate priority
field is a second source of truth about urgency, which disagrees with the first
the moment either is edited — D8's objection, one noun over.

### 3.3 R2 — No KV offload manager. This is the second time it has been proposed.

The idea is to evict an idle or blocked task's KV so another task can have the
slot, then reinstate it when the owner answers. It should not be built, for
three reasons in ascending order of force:

- **The server already does it.** `-cram` *is* offload-and-reinstate, sized and
  measured (§3.1).
- **It composes out of D13 for free.** A question that ends the run releases the
  slot at the turn boundary and leaves the prefix in the prompt cache on the way
  out; the resume hours later restores it. The behaviour is emergent, not
  engineered. Worst case the cache has evicted and the resume re-prefills — a
  latency cost, not a correctness one.
- **The hand-rolled version was tried and reverted**, and the flag carries a "do
  not add it back" comment. `--cache-idle-slots`, added 2026-08-20 on exactly
  this reasoning, saves an idle slot *and clears it* — so a live conversation's
  prefix is wiped, LCP similarity finds nothing, and slot selection falls through
  to LRU onto a cold slot. Measured over one TUI session: 3 of 25 turns
  re-prefilled the whole transcript, the worst costing **20.5s for 29,570
  tokens**. Removed, the same test gave 1 LRU selection in 44.

The general lesson recorded beside that flag is what makes this a decision rather
than a preference: **a throughput benchmark cannot see the regression.** It sends
independent prompts, which is precisely the workload with no prefix to lose.
Anything proposed here has to be measured by prefix reuse across turns, not
tok/s.

Written down as a refusal rather than left as an omission, because a dropped idea
is free to come back tomorrow — the same reasoning that has the harness brief
carry every prior candidate as "already tried".

**And the comparison that motivates it does not hold.** Claude Desktop is cited
as doing something like this; it is a cloud client with no local model, so there
is no local KV to manage. What makes a returning conversation fast there is
server-side prompt caching with a TTL that simply expires — a simpler mechanism
than eviction, and not one there is anything here to copy.

### 3.4 R3 — Measure with the cache lens before building anything

`cache_lens.rs` is the per-run observer that caught the `--cache-idle-slots`
regression by name ("prompt cache reuse dropped: re-paid 15733 input tokens").
Pointed at resumed task sessions it answers the one question §3.3 leaves open:
does a conversation parked overnight actually get its prefix back, or has a
night of triggers and chat pushed it out of 32 GB?

That is a measurement, not a build, and it is the thing to do first. If parked
sessions miss, the answer is a larger `-cram` before it is anything cleverer.

---

## Phases

**Phase 1 — the CLI verb. Built 2026-08-26.** `mecha tasks work <id>
[--note ...]`: seeded run, own session, outbox-bound, prints the session id
and what it staged. Carried D14 and D15 with it, because this verb is the
first thing to put a second conversation on one agent and the first to
resume one.

Three things the first live run taught, none visible from the code. The run
was **non-interactive and could therefore do nothing** — it made three
`fs_write` calls, had all three blocked, and reported back the contents of
files it had not been allowed to create; `mail draft`'s `prepare(_, false)`
is right for a run whose only outbound act stages, and wrong for one that
does work. The session **recorded no config**, so the withheld
`kg_task_update` was a claim rather than evidence and `replay` could not
rebuild the surface. And `withhold_tool` **could not reach subagent
registries** — `build_subagent` clones tools out of the pool during
`prepare`, so a profile allowlisting the withheld tool left delegation as
the way around D6; both entry points now refuse to start rather than strip
silently.

**Phase 2 — the question store (D13). Built 2026-08-26.** A question ends
the run and waits; an answer resumes the session. `/queues` gained the row
and doctor watches it at 24h — shorter than the outbox's 48h, because a
pending draft is finished work sitting safely while an unanswered question
is a delegation frozen mid-flight.

**Phase 3 — the graph side. Built 2026-08-26.** `waiting_on` and `session`
on `kg_task_update`, the `agent-mecha` node, `@owner` (D8, D9, D9a, D9b).
Two traps worth keeping: the first cut **retired the old `waiting_on` before
resolving the new name**, so a typo did not merely fail — it cleared who
actually had the ball; and **seeding a node in a migration broke encrypt,
decrypt and fork**, because the target runs migrations before the copy and
already held the row the source was about to send. `nodes` joined the
`INSERT OR IGNORE` pass `predicate` has always been in.

**Phase 4 — the phone. Built 2026-08-26; D12 superseded, B1 and B2 amended
and shipped.** *Ask mecha* on the task row, *open the conversation*, *stop*,
the drawer filter and chip (D10), the rendered todo list, and — in a second
pass the same day — the **return path** and the derived card states (D16).
D12 was decided against as written (see the decision itself) and its cheap
half — front-loaded questions in the seed — shipped in its place. B1 and B2
closed that evening, both **amended first**: re-reading them against the
shipped row changed both, and the amendments sit beside the originals.

**And in an eighth pass the same day, *ask mecha* stopped being a
fire-and-forget child at all** — it opens the task's chat session, the model
speaks first, and the board does not move until the owner hands it over. That
is D2 restored rather than a new decision; the mechanism and the four rules
it needed are in `CLAUDE.md`'s task-board section, and HISTORY has the
narrative.

The return path was not on this list and is the half of D13 the first pass
left implicit: a question could be *asked* from the phone's delegation and
answered only from a terminal, so the gesture the phone exists for opened a
loop the phone could not close. `/api/questions` list/answer/abandon, with
the card on its task — D13's own "no new noun" argument — and answering
spawning the resume detached, `--unattended`.

Two findings from building it, both recorded in HISTORY under Traps and
worth knowing before touching this arc again:

- **`questions answer` built an interactive agent.** Detached with
  `/dev/null` on stdin, that files every approval as
  `"Denied by the user: "` — the string the learning miner reads a
  *correction* out of. The flag is a precondition for the web reaching the
  verb at all, not an ergonomic.
- **No task run had ever written a `RunStats`.** `tasks work` and
  `questions answer` were the two front-ends missing `record_outcome`, so
  the corpus this document's own "task shape" question defers to had never
  seen a delegation. It has, from 2026-08-26.

*Verified: tap → a session appears in the drawer → opening it shows the plan
→ steering works → stop works → a failed run reads as failed and not as
idle. Not yet verified, because D12 is unbuilt: editing the plan changes
what runs.*

**Phase 5 — narrowing and context.** The context assembler (D4) — shipped
2026-08-26, as a **pointer** rather than an assembler; see D4's amendment.
D6's narrowing arrived early with Phase 1, because shipping a task runner
that could close its own task was not an option worth a phase boundary.

**Phase 6 — admission control (R1). The only phase still unbuilt, and now
due.** It was deferred on a stated trigger — *worth building once more than
one delegation at a time is routine, and not before*, since with four slots
and one owner the contention it manages may simply not arise. The eighth pass
is what makes it routine: a delegation is now a conversation the owner opens
by tapping, it carries its own 200-turn ceiling, and it runs on the same
llama-server as chat, voice and the trigger daemon. Nothing decides what runs
when, and answering a parked question spawns its resumed run immediately —
which is the half the owner has already asked for by name (*"when the
question is answered it should resume in the queue"*). R3's measurement is
still the input, and the sharper version of the cost question below is now
the argument.

## Open at design time

- **Does the prompt cache hold overnight?** §3.4's measurement. Everything in
  R2 rests on a parked session getting its prefix back after a night of
  triggers and chat, and that has never been measured — only the absence of
  evictions at 32 GB under today's traffic, which is not the same claim.
- ~~**Should the plan gate be skippable?**~~ **Moot as of 2026-08-26**: D12
  was decided against as written, so there is no gate to skip. What replaced
  the question is a narrower one — whether a *reviewable plan document*,
  separate from the todos, is worth building — and it has its own query
  waiting on the corpus (delegations that ended `ready for review` and were
  then dropped or reworked). Kept rather than deleted, because the next
  reader will otherwise re-propose the gate.
- **What does the owner see while it runs, on a sleeping phone?** Still push
  (remote-surface Phase 5), still unbuilt. Narrowed twice since: D13 means a
  run that ends on a question needs no live channel to be useful, and the
  eighth pass means a question asked with nobody watching **parks** rather
  than expiring — so the sleeping phone now costs a delay rather than a
  refused call. What is left is genuinely the notification.
- **Cost.** Every *ask mecha* is a full agent run on the local model, and there
  is no per-task accounting — `[agent] budget` is per run. Devin's most-cited
  complaint is exactly this opacity: with async delegation "you may not know the
  ACU tab until the invoice arrives." Local inference makes the money question
  moot and the **time** question sharper: the model is shared with chat, voice
  and triggers, so a task run is latency somebody else pays for. Worth measuring
  before the gesture is cheap enough to tap idly.
- **Task shape.** Copilot's success rate ran from 84.7% (cleanup) to 54.5%
  (performance work, which it cannot self-validate). The equivalent question
  here — which kinds of board item are worth delegating — can only be answered
  by running it, and `RunStats` already records enough to answer it later.
