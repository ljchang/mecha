# Tasks: how everyone else does it

Research for two questions the owner asked on 2026-08-24, after a day using
the phone app's Tasks page:

1. The board's own organization — *"I don't really get what this is. is it a
   way to organize tasks? If so, maybe we can research other apps to see how
   they do this including the Getting things done philosophy."*
2. Handing work to the agent — *"I can't assign it to an agent or prompt an
   agent to do something to help complete a task."*

`TASK-AGENT-DESIGN.md` is what this argues for. Findings only here; every
claim carries its source, and where products disagree that is said plainly,
because **the disagreements are the design decisions**.

---

## Part 1 — Task organization (Things 3, OmniFocus 4, Todoist, TickTick)

### 1.1 The sidebar, and what derives each bucket

| App | Fixed buckets | Derivation |
|---|---|---|
| **Things 3** | Inbox, Today (+ This Evening), Upcoming, Anytime, Someday, Logbook, Trash | Mixed: Today = start date **or** deadline **or** repeat matching today; Upcoming = future *start* dates; **Anytime and Someday are status**, not dates |
| **OmniFocus 4** | Inbox, Projects, Tags, Forecast, Flagged, Nearby, **Review**, Completed, Changed + unlimited custom perspectives | Filter primitives: availability (Available / First Available / Remaining / All), status (Active / **On Hold** / Completed / Dropped), defer, due, tags, flagged. Projects *and* tags each carry their own status |
| **Todoist** | Inbox, Today, Upcoming, Filters & Labels | All three fixed views are date-derived; everything status-like is a user-written query (`p1 & @delegated & #Work`) |
| **TickTick** | All, Today, Tomorrow, Next 7 Days, Inbox, **Assigned to Me**, **Won't Do**, Completed | Toggleable smart lists |

Sources: [Things date logic](https://culturedcode.com/things/support/articles/4001304/) ·
[OmniFocus perspectives](https://support.omnigroup.com/documentation/omnifocus/universal/4.3.3/en/perspectives/) ·
[Todoist glossary](https://www.todoist.com/help/articles/todoist-glossary-cA60laWMH) ·
[Todoist filters](https://www.todoist.com/help/articles/introduction-to-filters-V98wIH) ·
[TickTick smart lists](https://help.ticktick.com/articles/7055792921664028672)

**The disagreement to decide from: how many fixed buckets, and whether
"someday" is a status.** Things ships six opinionated views and *no*
user-defined ones. OmniFocus ships nine plus unlimited custom perspectives.
Todoist ships three and a query language. Someday exists as a status in
Things, as *On Hold* in OmniFocus, and not at all in Todoist/TickTick (a
label, or an undated task).

Two specifics worth stealing outright:

- **Things splits start date from deadline.** A *start* date puts a task in
  Today; a *deadline* is a separate field that does not. This is the fix for
  the failure mode below.
- **TickTick's "Won't Do"** is an explicit abandon state distinct from
  complete — OmniFocus's *Dropped* by another name. Both exist so that
  "decided not to" is not recorded as "finished".

### 1.2 GTD canon, and where the apps leave it

Canon: **Next Actions** (by context), **Projects** (an index of multi-step
outcomes), **Waiting For**, **Someday/Maybe**, and the **Calendar** — which
holds *only* date/time-specific commitments, the "hard landscape"
([gtd.be](https://www.gtd.be/en/what-is-gtd/the-5-steps-of-gtd)).

Every deviation runs the same direction — **apps replace status with dates**:

- Todoist and TickTick's *Today* means "everything due today", so people
  date things they merely *want* to do and Today becomes a wish list. Canon
  says the calendar is sacred; the apps made it a to-do list.
- **"Waiting For" is not a built-in list in any of the four.** Every app
  makes the user construct it from a tag or a filter.
- Contexts became tags (OmniFocus renamed Contexts → Tags in v3, allowing
  many per task), losing one-context-at-a-time.
- **Only OmniFocus ships Review** as a first-class perspective, with
  per-project review intervals — the weekly review, in the product.

### 1.3 Row actions: what is one gesture and what opens a sheet

- **Things**: swipe right = When (schedule); swipe left = select for batch.
  Complete is a tap on the circle only.
  ([source](https://culturedcode.com/things/support/articles/2803582/))
- **Todoist**: swipe actions are user-chosen from Complete, Schedule,
  Delete, Reminders, Select.
  ([source](https://www.todoist.com/help/articles/how-to-change-your-swipe-actions-D5DQOQz6))
- **TickTick**: four slots (short/long × left/right) from None, Complete,
  Due Date, Priority, Move to, Delete.
  ([source](https://blog.ticktick.com/2016/04/25/customize-your-swipe-options-in-ticktick-275/))

**The invariant across all three: complete and schedule are one gesture;
move, delegate and edit open a sheet.** Scheduling is the only value-setting
action anyone puts behind a swipe, and it opens a mini date picker rather
than a full form.

### 1.4 Delegation is modeled two incompatible ways

- **Assignee** (multi-user apps): Todoist `+name` within a shared project,
  with `assigned to: others` / `assigned by: me` as the delegated views;
  TickTick `@user` plus the *Assigned to Me* smart list.
  ([Todoist](https://www.todoist.com/help/articles/search-for-tasks-assigned-to-others-D5DbUlSS))
- **Waiting-for tag** (single-user apps): the OmniFocus convention is a
  `waiting` tag whose **tag status is On Hold**, which structurally removes
  it from Available views, surfaced by a custom Waiting For perspective.
  Things has no assignee concept at all.
  ([source](https://discourse.omnigroup.com/t/on-hold-vs-waiting-perspectives-tags/70260))

**The transferable insight is OmniFocus's:** delegation is *a status that
suppresses the item from "what can I do now" while keeping it reviewable*.
An assignee field alone does not do that.

### 1.5 Capture

Natural-language date parsing is table stakes in Todoist/TickTick and
deliberately narrower in Things/OmniFocus:

- **Todoist Quick Add** parses "tomorrow at 3 PM" inline and highlights the
  token; `#project`, `@label`, `+assignee`, `p1–p4`, `/section`.
- **TickTick**: `^list`, `#tag`, `*date`, `!priority`, `@user`, plus free
  text — "buy ticket tomorrow 8 am" sets due date *and* reminder.
  ([syntax](https://curtismchale.ca/2020/08/10/ticktick-quick-add-syntax/))
- **Things** Quick Entry is global (`Ctrl-Space`), and *Quick Entry with
  Autofill* captures a link to the current Mail message / Safari page /
  Finder file into the note. NL dates work only inside the date picker, not
  in the title.
  ([source](https://culturedcode.com/things/support/articles/2249437/))
- **OmniFocus**: Quick Entry, **Mail Drop** (email → Inbox, subject = title,
  body = note, with per-address privacy), SiriKit.
  ([source](https://support.omnigroup.com/documentation/omnifocus/mac/4.0/en/capture-methods/))

**Disagreement:** whether parsing mutates the title. Todoist/TickTick strip
the parsed date out of the name; Things keeps the name literal.
**Invariant:** capture lands in Inbox and organization is deferred — no app
makes you choose a project at capture time.

---

## Part 2 — Handing a task to an agent

### 2.1 The gesture, and the one real disagreement

The assignee field won as the *gesture* nearly everywhere. What products
disagree about is whether assignment **transfers accountability**.

- **Linear redesigned the data model to say no.** Issues "can only be
  *assigned* to humans, and only *delegated* to agents" — the human
  assignee stays and the agent is added as a delegate. The stated reason is
  their **Principle 06: "An agent cannot be held accountable"** — "unlike
  when you assign an issue to a human teammate, the responsibility doesn't
  transfer." The practical reason is as sharp: before delegation existed,
  "you'd sometimes see an agent with dozens of issues assigned, but no clear
  sense of who was behind them. If you disagreed with what the agent was
  doing, it wasn't obvious who to talk to." Two gestures start work — set
  the delegate, or **@mention the agent in a comment** — and both mint an
  `AgentSession`.
  ([AIG](https://linear.app/developers/aig) ·
  [approach](https://linear.app/now/our-approach-to-building-the-agent-interaction-sdk) ·
  [docs](https://linear.app/docs/agents-in-linear))
- **GitHub Copilot: assign it exactly like a person** — "just like you would
  with a human software developer", from github.com, mobile, or `gh`.
  ([source](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/kick-off-a-task))
- **Asana AI Teammates** (Fall 2025): assigned like any team member, but the
  teammate is first **added to a project with a defined scope** — the agent
  is a project member with a role, not a global bot.
  ([source](https://asana.com/inside-asana/fall-release-2025))
- **Devin**: no tracked-item gesture of its own — @mention in Slack, or take
  tasks from Linear/Jira — then produces an "Interactive Planning" blueprint
  **with a confidence score** before writing code.
- **Claude Code**: `&` prefix sends a run to cloud infra; `/teleport` pulls a
  cloud session back into the local terminal — notable because the handoff
  is *reversible*.
  ([source](https://code.claude.com/docs/en/claude-code-on-the-web))

### 2.2 Where the work appears, and how progress is shown

- **Copilot** always opens a **draft PR** immediately, adds an 👀 reaction as
  instant receipt, pushes commits as it goes, and streams logs behind "View
  session". The output is a PR, never a comment.
- **Linear** puts an `AgentSession` thread on the issue, carrying typed
  `AgentActivity` records: `thought`, `action`, `elicitation`, `response`,
  `error`. Session state (`pending`, `active`, `error`, `awaitingInput`,
  `complete`, `stale`) is **derived by Linear from the last emitted
  activity — the agent never sets it directly.** An agent must emit
  something within **10 seconds** of the created event or be marked
  unresponsive.
  ([interaction docs](https://linear.app/developers/agent-interaction))
- **Asana** decomposes the task into **subtasks** as the visible plan,
  reviewed at "transparent checkpoints".

### 2.3 Review, and mid-run questions

- **The draft PR is Copilot's disposal surface**: it "asks for your
  review… if you leave feedback, it'll revise the PR and keep going until
  you approve." Structurally, Copilot **cannot self-approve** its PRs and
  Actions require human approval to run on them — a gate, not a prompt.
- **ChatGPT agent mode** named two patterns worth keeping in vocabulary —
  *confirmation before consequential actions*, and *Watch Mode* (sending
  mail or submitting a form requires active oversight, with pause and
  take-over). Note it shipped and was **withdrawn**; OpenAI's help centre
  now says the agent "is no longer available".
  ([OpenAI](https://openai.com/index/introducing-chatgpt-agent/) ·
  [NN/g](https://www.nngroup.com/articles/impressions-chatgpt-agent/))
- **Linear is the only one with a first-class mid-run question**:
  `elicitation` puts the session in `awaitingInput`, and the user's answer
  arrives as a `prompted` webhook — answered from the item's own UI. Their
  Principle 05 additionally requires agents to **respect disengagement** and
  resume only on explicit re-engagement.
- **Copilot has no mid-run question and no stop button.** A stuck session
  "will time out after an hour"; the documented recovery is to **unassign
  and reassign**. Firewall blocks are appended to the PR body.
  ([troubleshooting](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/coding-agent/troubleshoot-coding-agent))

### 2.4 The cautionary numbers

Microsoft ran the Copilot coding agent in `dotnet/runtime` for ten months
(May 2025 – Mar 2026), and published the audit
([source](https://devblogs.microsoft.com/dotnet/ten-months-with-cca-in-dotnet-runtime/)):

- **878 PRs, 535 merged = 67.9%**, against 87.1% for Microsoft humans and
  79.7% for community contributors.
- **Autonomy is oversold**: **52.3%** of agent PRs needed direct human
  commits. With intervention 86.2% succeeded; **fully autonomous, 55.1%**.
- **Setup dominates outcome**: 38.1% success before `copilot-instructions.md`
  tuning, **69% after**.
- **Task shape matters**: cleanup 84.7%, tests 75.6%, bug fixes 69.4%,
  performance 54.5% (it cannot validate its own performance claims). Sweet
  spot: 1–50 line diffs.
- **Review burden is measurable**: merged agent PRs drew **16.5 review
  comments vs 12.4** for human PRs; median time-to-merge 50 hours.
- Quality holds where it merges — **0.6% revert rate vs 0.8%** human — but
  65.7% of added lines were test code.
- **Cost was explicitly not analyzed**, though conceded to be real.

Ecosystem-wide, **more than 1 in 5 code reviews on GitHub now involve an
agent**: PR throughput scales while human review capacity does not
([GitHub](https://github.blog/ai-and-ml/generative-ai/agent-pull-requests-are-everywhere-heres-how-to-review-them/)).
A Jan 2026 study found agent-generated changes carry more redundancy and
technical debt per change, hidden behind passing tests and tidy diffs.

**Devin's distinct complaint is cost opacity**: ACU billing has no
cross-product benchmark, and "if your team is assigning Devin tasks through
Slack asynchronously, you may not know the ACU tab until the invoice
arrives"
([critique](https://brainroad.com/devin-pricing-in-2026-real-cost-hidden-spend-and-alternatives/)).

### 2.5 What mecha should take

- **Linear's delegate-vs-assign split** is the citable accountability
  argument, and it matches a rule this project already enforces one store
  over (`ladder.rs`: a lane must not promote itself).
- **Typed activities with host-derived state** makes "is work happening?"
  answerable *without the agent self-reporting*, which is the same reasoning
  that makes mecha read a trigger's last answer from the session transcript
  rather than a cached copy.
- **`elicitation` / `awaitingInput`** is the only shipped answer to mid-run
  questions from an item's UI — and mecha already has the machinery
  (`ask_user`, `serve/present.rs::Questions`, pending cards riding the
  transcript read).
- **Copilot's missing stop button and 52.3% intervention rate** are the
  cautionary data: a task run must be cancellable, and the design should
  assume a human joins the loop rather than hoping they need not.
