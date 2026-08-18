# Managing mail from mecha — research

*2026-08-17. What the field has converged on for agent-driven email, what
flowmail already learned, and what mecha would actually have to build. The
survey behind a `/mail` triage surface, written before the design so the
design argues from evidence rather than from the one inbox it grew out of.*

---

## 0. The short answer

Almost all of this exists already, in the wrong shape or one directory over.

The outbox is the approval queue. The front door is the triage queue — its
state machine, its quarantined extractor, its reply-into-outbox join, and its
TUI modal are all the shape a mail triage surface needs, built and shipped.
The learning store is the correction loop, and a stronger one than flowmail's,
because a mail correction would ride the validation ledger instead of going
straight into a prompt. pkg holds the people, the tags and the task board, and
`kg_entity` already resolves an email address to a person node.

Three things are genuinely missing, and only the first is a blocker:

1. **`mecha-mail` cannot express `archive`.** Or read, or spam, or label. The
   entire write surface is send, reply, and calendar CRUD. A triage UI over
   this surface can move the cursor and draft; it cannot empty an inbox.
2. **There is no triage store**, so every look at the inbox is a fresh model
   pass over other people's prose, at the moment you are least willing to wait.
3. **There is no `/mail`**, which is the smallest of the three and the last
   one that should be written.

A fourth thing is missing and is not strictly part of this feature: mecha has
no **skills** mechanism — no way to write a procedure as prose in a file that
the model can pick up by name. The mail categories are the clearest case for
one, so §9 sketches it, but it wants its own design document.

And one finding that reframes the feature. Four of the five mail categories
this is being built for — recommendation letters, lab applications, meeting
requests, speaking invitations — **already have typed manifests in
`mecha-manifest/types/`**, because the front door was built for exactly those
requests arriving through a form. An email asking for a letter is not a new
kind of thing. It is a `letter` request that arrived through the wrong door,
untyped. That makes the highest-value classification verb not "label this" but
**"recognise this as an existing request type and route it to the front
door"** — where the extractor, the deadline field, the `needs-info` park, the
draft-into-outbox and the reconcile loop are already written and already
guarded.

---

## 1. What is being asked for

Verbatim, from the conversation that prompted this:

- A TUI for quickly going through mail.
- A classifier that proposes what needs to be done, which can be corrected.
- Read an email, create a task from it that the agent can then help solve.
- Take an email and put the event on the calendar.
- Archive. Mark as spam.
- **Not** flowmail's cards. Instead: tags or nodes that automatically label
  mail — both so it can be found later, and so the tag carries what has to
  happen next.

With five worked examples, which are the real specification:

| The mail | What it needs |
|---|---|
| Receipts and expenses | Tagged, and forwarded to the finance person |
| Undergrad / grad / postdoc asking to join the lab | The same handling as the factory's application form |
| Rec letter requests, and mail from the schools | Tagged, tied to the right *person*, and **no school missed** |
| Department and university mail needing review | Tagged, and on the task board so it actually gets done |
| Everything else | Out of the way |

Note what the third one is. "Make sure no school is missed" is not a labelling
problem — it is a *completeness* problem over a set of obligations that arrive
as unrelated messages across weeks. That one example is the reason a tag alone
is not enough and the task board has to be in the loop.

---

## 2. The landscape

Four families, and they disagree about almost everything except one thing.

### Agent Inbox (LangChain)

The closest structural analog. Its contribution is a **closed set of four
responses** to any agent-proposed action — `accept`, `edit`, `respond`,
`ignore` — plus a per-item config saying which of the four this item permits
(`allow_accept`, `allow_edit`, `allow_respond`, `allow_ignore`). The stated
rationale is graduated intervention rather than binary approve/reject, and a
set small enough that the interface never has to teach you anything.

What to take: the closed enum, and the per-item permission. What to leave: it
conflates the triage queue with the approval queue, which is why using it is
confusing — you approve a draft in the same list where you decide whether a
thing needs a draft at all. mecha must not do this; the outbox already is the
approval queue.

### executive-ai-assistant / agents-from-scratch (LangChain)

Settles the *classification* side on three buckets: `IGNORE` (not worth
responding to or tracking), `NOTIFY` (worth knowing, no response), `RESPOND`
(needs a direct answer). Three, not twelve. Memory is split three ways —
semantic (facts about the user), episodic (few-shot examples injected at the
triage step), procedural (the system prompt, optimised from feedback).

What to take: three buckets is enough, and the triage step is where few-shot
correction examples belong. What mecha already does better: all three memory
kinds exist here with provenance gating on top, which that system has no
concept of.

### Inbox Zero (open source)

The rules-engine take. Plain-English rules — "Cursor Rules for email" — an LLM
classifier, and a rule engine that executes label / archive / draft. The design
decision worth stealing: **human-in-the-loop by default, with per-rule
graduation to autonomy.** You promote one rule at a time from propose to
execute as you come to trust it. Not a global autopilot switch.

### Superhuman / Shortwave / the TUI clients

Ergonomics, not architecture. One-key triage, a command palette for
everything, split inbox, sub-100ms. The honest framing from the comparisons:
*Superhuman thinks email is too slow; Shortwave thinks email is too dumb.*
mecha's problem is the first one — the model is already here. aerc and
Himalaya are the terminal baseline this will be compared against: vim keys, an
ex-command line, and in Himalaya's case a deliberate statelessness that has
made it a popular backend for exactly this kind of agent triage.

### The convergence

Every one of these independently arrived at **a small closed action set over a
durable queue of proposed actions**. That is what falls out whenever a model
proposes and a person disposes. mecha has already built that twice — the
outbox (`pending` → `sent`/`rejected`, with `OutboxKind`) and the front door
(`drained` → `extracted` → `triaged` → `awaiting_me` → `answered`/`closed`).
This feature is a third instance of a pattern the repo owns, not new
machinery.

---

## 3. flowmail, read against mecha

flowmail was a Tauri/Svelte desktop app built around cards. Its real
inventions, from `dev_docs/AI_PIPELINE.md` and `CORRECTION_SYSTEM.md`:

- **A two-model split** — Haiku triages at roughly $0.001/email, Sonnet
  drafts. Cheap classification on everything, expensive generation only on
  demand.
- **A deterministic Context Assembler** between them, with no LLM in it. Seven
  ordered sources: card notes, thread history, contact tone, the card's system
  prompt, user preferences, previously-rejected drafts, memory facts.
- **Cards** as the context primitive — a mental bucket carrying a
  `system_prompt` and auto-assign rules.
- **Three-timescale correction**: reflexion (immediate, causal, 2–4 sentences
  on *why* the classifier was wrong), abstraction (~10 reflexions → rules),
  consolidation (~20 rules → holistic rewrite).
- **The Focus Queue's GTD rule** — every item must have a clear next action,
  and dismissing an item *with a reason* is what taught the extractor to stop
  generating that kind of item.

Read against this repo, most of it is already here and most of what is here is
stronger:

| flowmail | mecha's equivalent | Which is better |
|---|---|---|
| reflexion → abstraction → consolidation | `learning.rs`: reflect → learn → validate | mecha, and not close — provenance gating, the validation ledger, gated retirement, a hard cap on active rules |
| `drafts` table, `pending → approved → sent` | `outbox.rs` | mecha — taint snapshots, recorded release jail, staging that fails closed |
| edit diffs feed the drafter | the `writing` domain mines `diff(args_before, args)` | equivalent; mecha's is already wired |
| memories, fact candidates, a review queue | pkg over MCP | pkg — bi-temporal, polarity, contradiction flags |
| Focus Queue tasks | `kg_task_*`, which `morning` already reads | pkg |
| Triage agent → `{priority, category, needs_response, …}` | **nothing** | flowmail |
| Context Assembler | **nothing** (the agent loop improvises) | flowmail |
| cards | **nothing** | see below |

So the port is not "rebuild flowmail in a terminal." It is: build the two
things mecha lacks, and wire the rest to machinery that already exists and is
better.

### Cards, and why they should not be rebuilt

A card did three jobs: it grouped mail, it carried drafting instructions, and
it was the unit the UI navigated. In mecha those three have separate owners
already — pkg entities group, learned rules and subagent profiles instruct,
and the TUI navigates a store. Rebuilding cards would create a fourth place
that knows about people and a second place that shapes prompts, which is how
you get two sources of truth about the same colleague.

The user's instinct — *tags or nodes, not cards* — is the correct decomposition
of the card into its two useful halves. Section 5 takes it seriously.

---

## 4. Where mecha actually stands

### What exists and is directly reusable

- **`outbox.rs` + `/outbox`** — the approval queue, with confirmation, taint
  display in red, edit, reject-with-reason, and a release that rebuilds the
  tool surface at the recorded jail.
- **`frontdoor.rs` + `/frontdoor`** — the full triage state machine, the
  quarantined extractor with no tools and no history, `triage` drafting into
  the outbox, `needs-info`, `close` requiring a reason, and `reconcile` that
  runs on its own rather than on a verb you have to remember.
- **`trigger.rs` + `cron.rs`** — the scheduler. `morning` already reads mail
  and the task board every day at 07:00, read-only, with an explicit tool
  allowlist.
- **`learning.rs`** — reflect / learn / validate, with `Origin` gating.
- **pkg over MCP** — `kg_entity` resolves *a name, alias, or email address* to
  a person node with facts and interaction recency; `kg_search` filters
  episodes by user-applied `#tag`; `kg_task_create` captures a task with
  `name`, optional `due`, `context` (a GTD tag like `@email`), and `project`
  which must resolve to an existing node.
- **The TUI modal pattern** — five instances (`/triggers`, `/outbox`,
  `/frontdoor`, `/polls`, `/doctor`), all following the same rules: read the
  store for display, shell out to `mecha …` for every mutation, spawn slow
  work detached and poll the store rather than the child.
- **`mecha-manifest/types/`** — `letter`, `lab-application`, `meeting`,
  `speaking`, `book`. Discussed in §6, because this is the finding.

### The write surface, in full

`mecha-mail/src/unified.rs:1371-1383` is the whole vocabulary:

```
reads    mail_search   mail_recent   mail_get_thread
         calendar_list  calendar_list_events  calendar_freebusy
writes   mail_send     mail_reply
         calendar_create_event  calendar_update_event  calendar_delete_event
```

No archive, no mark-read, no star, no label, no move, no trash, no spam.
`mecha-mail/src/google/gmail.rs:3` says so in the module docs — *"no
spam/trash/archive ops"* — so this is a known omission rather than a
regression.

Two smaller gaps on the read side. `gmail.rs:332-361` parses `labels`,
`is_read` and `is_starred` off every message and the unified layer flattens
them away, so a caller cannot ask for unread-only. And Graph has no labels at
all (`graph_mail.rs:309`), which any labelling verb has to face honestly
rather than paper over.

---

## 5. The three things to build

### Layer 1 — the verbs, and a capability quadrant that does not exist yet

Add to `unified.rs`, account-scoped like every other item operation:

| Tool | Gmail | Graph |
|---|---|---|
| `mail_archive` | remove `INBOX` | move to Archive |
| `mail_mark_read` / `mail_mark_unread` | toggle `UNREAD` | `isRead` |
| `mail_spam` | add `SPAM`, remove `INBOX` | move to Junk |
| `mail_trash` | `trash` | `POST /messages/{id}/move` → Deleted |

Plus `unread_only` on `mail_recent`, and `is_read` / `is_starred` / `labels`
carried through to the caller.

Tagging is deliberately absent from this table. It is not a provider
operation at all — see the end of this section.

`mail_trash` stays separate from `mail_archive` on purpose: they are different
regrets, and a model that can only reach one of them should be able to reach
the reversible one.

`mail_spam` is worth its own verb rather than a special case of `mail_tag`.
Marking spam trains the provider's filter — it is the one triage action with a
side effect outside the mailbox, and folding it into a generic tag call hides
that.

**The capability labelling is the part to get right, because these land in a
quadrant the mail surface does not currently have.** Today it splits cleanly:

- *Reads* are `untrusted_input` sources but not sinks. A Gmail query reaches
  only googleapis.com, who already custodies the mailbox — hence `readOnlyHint`
  and no `openWorldHint`, which is the whole difference from `http_fetch`.
- *Sends and calendar writes* reach third parties, carry `openWorldHint`, and
  are named in `[outbox] tools` so they stage.

Archive, read, spam, tag and trash are **neither**. Nothing leaves the machine
— no third party learns anything — so they are not `external_send`, and they
must **not** go in `[outbox] tools`. Staging them would make triage pointless:
you would review a queue in order to fill another queue. But they mutate the
user's own state, so they are `destructive: true`, `private_data: true`, and
they belong to the approver.

Getting this wrong in either direction is expensive, and in opposite ways.
Marking them `external_send` makes the whole feature unusable. Marking them
`read_only` means an unattended trigger running `permission_mode =
"read-only"` — which is exactly what `morning` runs as — could quietly start
emptying the inbox at seven in the morning.

#### The actual blocker is OAuth scope, on both providers

This is the part that has to be settled before any of the above is written,
and it is not a code question.

**Neither account can currently modify a message, by deliberate design.**

```
microsoft/auth.rs:26   Mail.Read  Mail.Send  Calendars.ReadWrite  offline_access
                       "`Mail.ReadWrite` is deliberately absent — nothing here
                        modifies a message in place, and least privilege beats
                        a future consent click."

google/auth.rs:79      gmail.readonly  gmail.send  calendar  calendar.events
google/auth.rs:369     #[test] scopes_cover_mail_and_calendar_but_not_modify
```

There is a *test asserting the current state*. This was a considered decision,
and archive/spam/read is exactly the "future consent click" it anticipated.
Revisiting it costs differently per provider:

- **Google** needs `gmail.modify`. It is a *restricted* scope — but so is
  `gmail.readonly`, which is already in use, so the marginal verification
  burden is near zero for a personal OAuth client. The real cost is that
  adding a scope invalidates the consent behind the stored refresh token:
  every existing account must re-authenticate. `mecha-mail auth` already
  handles that path, and `doctor` already reports dead auth, so the machinery
  is there. **Tractable.**

- **Microsoft** needs `Mail.ReadWrite`, and that is **admin-restricted**.
  Microsoft's recommended user-consent policy blocks it from end-user consent
  outright; a non-admin sees *"Need admin approval"* rather than a consent
  screen. Which means it cannot be turned on by the user — it needs the
  tenant's administrator to grant it to the app registration. And the whole
  reason device-code flow was chosen was so that an **org-approved app
  registration could be reused untouched**. Asking for `Mail.ReadWrite` gives
  that up and turns setup into an IT ticket. **Blocked on institutional
  policy, not on code.** (Worth noting for later: from 2026-12-31 Microsoft
  moves modification of *sensitive* mail properties behind a further
  `Mail-Advanced.ReadWrite`, so this ledge is still moving.)

The honest consequence: **`mail_archive` and `mail_spam` may ship
Gmail-only for some time.** That is survivable and should be built for
rather than papered over — a fan-out read already reports a failed account
beside the others' results rather than sinking the call, and the same
convention applies here. What must not happen is a verb that silently does
nothing on one account. `mecha doctor` should report an account whose scopes
cannot support the triage verbs, so "why did archive not work on Dartmouth"
is answerable without reading source.

#### Tags are internal, and that is the better design anyway

Graph has no labels — categories are the nearest object and they are not the
same thing (`graph_mail.rs:309`). Mapping onto them would make a tag mean
something subtly different per account, which breaks the one job a tag has.

But the framing was wrong. **A tag does not have to be a provider concept at
all.** It lives on the triage record in `~/.mecha/mail-triage/`, and that is
strictly better on four counts:

- **No scope, no consent, no admin.** Tagging works identically on Gmail and
  Outlook today, and is untouched by everything in the section above. It is
  the one part of the feature with no institutional dependency.
- **No provider divergence** to reconcile, now or when a third provider
  arrives.
- **Richer than a label.** A tag can sit beside an entity link, a proposed
  action, a deadline and a `request_type` on the same record. Gmail labels are
  strings.
- **The vocabulary stays mecha's.** A closed, correctable set that pkg's
  `#tag` filtering already understands, rather than whatever labels the
  mailbox has accumulated over fifteen years.

The one real cost, and it should be stated rather than discovered: **tags are
invisible in Gmail, Outlook and on your phone.** Mail triaged by mecha looks
untouched in every other client. Two mitigations, neither needed at first:
an optional *one-way mirror* of selected tags to provider labels for anyone
who wants them visible — gated on the same modify scope, and a projection
rather than a source of truth — or simply accepting that `/mail` and `mecha
mail list` are where tags are read.

Note what this does *not* solve. Archive cannot be internal: an inbox that
only empties inside mecha has not been emptied. So the scope question above
is unavoidable for archive, spam and read-state — it is now isolated to
exactly those three verbs instead of contaminating tagging as well.

### Layer 2 — the triage store, and a quarantined classifier

This is the load-bearing idea, and it is the front door's shape applied one
directory over.

Reading mail arms `untrusted_input` — mail bodies are other people's words, by
explicit design, and config forces the label. A triage loop that reads fifty
messages into one conversation arms the trifecta for all fifty, and every
draft it stages comes out tainted. That is *correct*, and at inbox scale it is
also useless: fifty red confirmations is fifty confirmations nobody reads. A
warning that fires on everything has stopped being a warning.

The front door already answered this exact question, in one sentence:

> **The privileged run sees the extraction, never the prose.**

Applied to mail: per message, run a **quarantined classifier** — no tools, no
history, one user message, an empty tool list, constructed the same way
`frontdoor`'s extractor is — that emits a typed verdict:

```jsonc
{
  "reasoning": "...",              // first, per the front door's note on
                                   // constrained decoding degrading reasoning
                                   // when the answer precedes the thinking
  "bucket":    "respond" | "notify" | "ignore",
  "urgency":   "now" | "today" | "week" | "none",
  "one_line":  "...",              // the list row, and the only prose that escapes
  "tags":      ["expense", "rec-letter"],
  "proposed":  "reply" | "archive" | "spam" | "schedule" | "task"
             | "forward" | "frontdoor" | "none",
  "deadline":  "2026-08-20" | null,
  "about":     "<sender address, for kg_entity to resolve>" | null,
  "request_type": "letter" | "lab-application" | ... | null   // §6
}
```

Written to `~/.mecha/mail-triage/` as one JSON record per thread — owner-only,
temp-sibling-and-rename, unknown fields preserved on write, the store rules
every other directory under `~/.mecha` already follows.

Five things fall out, and the fifth is the one that makes it worth doing:

- **The list view renders typed fields.** An injection in a subject line
  cannot reach the list, cannot reach a privileged run, and cannot reach a
  learned rule. `one_line` is the one prose field that escapes, which is a real
  residual and is why it is one line rather than a summary.
- **The classifier runs in its own throwaway `Conversation`**, so it never
  arms the main one — the same reason `frontdoor triage` gives each request a
  fresh conversation.
- **Opening `/mail` costs nothing.** A trigger classifies overnight; the modal
  reads a store. The common case — nothing new — costs zero tokens and no
  model at all, which is the same argument that kept `drain` out of `mecha
  frontdoor`.
- **The prose is still readable by you, deliberately.** `show` prints the
  body in the terminal, exactly as `frontdoor show` does, on the same
  reasoning: a person reading mail in a terminal is the safe context. You
  cannot be prompt-injected into mailing your own calendar somewhere.
- **It is gradeable.** A store of `(message → verdict)` with corrections on
  top is simultaneously an eval fixture, a `reflect` source, and the few-shot
  pool that executive-ai-assistant injects at the triage step. Classification
  accuracy stops being a feeling.

This is the *invert before declaring impossible* move, for the third recorded
time. The naive read is that mail triage is inherently trifecta-armed and must
therefore be permanently degraded. The inversion relocates the reading of
hostile prose to the untrusted side of a boundary and gives the privileged side
typed fields only — and it makes the feature better, not merely safer, because
a typed store is also what makes the list instant.

### Layer 3 — `/mail`

A sixth modal on the pattern the other five already follow. Two depths: the
list answers *what needs me*, the detail answers *what is this and what would
happen*.

```
 ┌ mail ─────────────────────────── 14 need you · 3 drafted · 2 parked ─┐
 │ ● now    Chen        #grant     budget revision — needs numbers      │
 │ ● today  Registrar   #teaching  room change for PSYC 51              │
 │   today  Ana         #writing   co-author draft, comments by Fri     │
 │   week   Kaplan      #rec-letter Yale portal link, due Sep 1         │
 │   week   Amazon      #expense   receipt, $412 — forward to finance   │
 │ ✎ drafted Kim        #lab-app   reply staged → /outbox               │
 └ r reply · a archive · s spam · e schedule · t task · f forward ──────┘
```

The action set is a **closed enum**, for the reasons `docs/SLACK-ACTIONS-DESIGN.md
§1` already argues at length. That decision was made once for Slack and should
be reused rather than re-derived.

| key | action | where it lands |
|---|---|---|
| `r` | reply | a full agent run on that thread → drafts into the **outbox** |
| `a` | archive | `mail_archive`, immediate, approver-gated |
| `s` | spam | `mail_spam`, immediate, approver-gated |
| `e` | schedule | `calendar_create_event`, staged; plus a reply if it needs one |
| `t` | task | `kg_task_create`, immediate — a task the user asked for is an instruction, not an inference |
| `f` | forward | a run that stages a forward into the outbox |
| `g` | tag | edit the tags on the record, immediate, no model |
| `n` | needs-info | park it, exactly as `frontdoor needs-info` does |
| `!` | correct | *this classification was wrong* → a `reflect` record |
| `enter` | detail | the prose, the extraction, the proposed action, the entity |

Three rules carried over from the existing modals:

- **`r`, `e` and `f` are agent runs, not keystrokes.** They build a tool
  surface and can take minutes, so they spawn detached and are *watched* by
  polling the store — never run on the event loop. `a`, `s`, `t` and `g` are
  single calls and can be synchronous.
- **The result of a reply lands in `/outbox`, not here.** There is exactly one
  approval surface and this is not it. `/mail` decides *whether* something
  needs an answer; `/outbox` decides whether *this* answer goes.
- **Every mutation shells out to `mecha mail …`.** One implementation, and no
  way for the TUI to do something the command line cannot.

`!` is the key flowmail proved is worth having. Dismissing with a reason is
what stopped its extractor generating junk. mecha's version is strictly
stronger: a correction here becomes a reflection that goes through provenance
classification, the proposal gate, and the validation ledger, rather than
straight into a prompt.

---

## 6. Tags, nodes, and the five categories

The user's decomposition — *tags or nodes, not cards* — is two mechanisms, and
they should stay two.

**A tag is a flat string on the triage record — mecha's own, never the
provider's.** Cheap, set by the classifier, correctable by hand with `g`, and
its job is findability plus routing. It should be a **small, mostly-closed
vocabulary** — an open set drifts into forty near-synonyms within a month and
stops being a filter. It aligns with pkg's existing `#tag` filtering on
episodes, so a tag applied here can be the same token used to search the graph
later. Keeping it internal is what makes it work identically across providers
and cost no OAuth scope; §5 has the full argument and the one real cost.

**A node is a pkg entity.** `kg_entity` already resolves a name, an alias, or
**an email address** to a person node with facts and per-channel interaction
recency. So "associate this with the correct person" is one existing call
against the sender address — not a new store, not a contacts table. The
classifier proposes `about`; resolution happens on the privileged side, where
a graph read belongs.

The split matters because they answer different questions. A tag answers *what
kind of thing is this*. A node answers *who is this about*. The rec-letter case
needs both at once and neither alone.

### The five categories, worked

**Receipts and expenses.** Tag `expense`. Proposed action `forward`, with the
finance recipient coming from config rather than the classifier — a model that
can choose the recipient of a forward is a model that can be talked into
choosing a different one. The forward stages through the outbox like any other
send, which is what makes this safe to propose automatically.

**Lab applications.** Tag `lab-app`. And here the finding: this is
`mecha-manifest/types/lab-application.toml`, which already exists, and whose
`stage` field is already exactly `undergraduate` / `masters` / `phd` /
`postdoc` — the user's own categories, typed, a year before this conversation.

**Rec letters and the schools.** Tag `rec-letter`, node = the student. This is
the hardest of the five and the most instructive. `letter.toml` already models
it: a `deadline` date, a `programs` multi-select capped at twelve, a
`submission_method` enum distinguishing *a portal emails me a link* from *I
email it somewhere*, and a `portal_note` that exists specifically because some
portals expire the link.

But *"make sure no school is missed"* is not something a tag can answer. Twelve
schools produce twelve portal emails over six weeks, each individually
unremarkable, and the failure mode is silence — the same failure mode the front
door exists to fix. The mechanism that answers it is the **task board with a
project**: `kg_task_create` requires `project` to resolve to an existing node,
so one letter obligation becomes one project node and each portal email becomes
a task under it. Completeness is then a board query, not a memory.

**Department and university mail.** Tag `admin` or `service`. Proposed action
`task`, `context = "@email"`, `due` from any deadline the classifier found.
This is the plainest case and the one that most needs the classifier to be
*conservative*: over-tagging turns the board into the inbox it was supposed to
drain.

**Everything else.** `bucket = ignore`, proposed `archive`, and it never
appears in the list unless asked for.

### The finding: four of five already have a manifest

`mecha-manifest/types/` contains `letter`, `lab-application`, `meeting`,
`speaking`, and `book`. The front door was built to receive exactly these
requests, typed, through a form — with per-field kinds, a `[verification]`
block proving the requester controls the address, multi-step flows, file
uploads, and `retain_days`.

So an email asking for a letter is not a new kind of object. **It is a
`letter` request that arrived through the wrong door, untyped.** Which makes
the highest-value classification output not a tag at all, but a
`request_type`: recognise the email as an instance of an existing manifest and
hand it to `frontdoor`, where every downstream verb is already built —
extraction into typed fields, `needs-info` when the deadline is missing,
`triage` drafting a reply into the outbox, `reconcile` closing the loop when
the reply is sent, and `close` requiring a reason.

Three consequences worth stating plainly, because they are the argument for
building the routing before building anything clever in `/mail`:

- **The reply for these categories is already designed.** Whatever the factory
  form's triage does with a `letter`, mail should do identically. One
  behaviour, two doors.
- **`needs-info` becomes the answer to a vague email.** "Can you write me a
  letter?" with no deadline and no programme list is exactly what the form's
  required fields exist to prevent. Routing it to the front door means the
  reply that asks for the missing pieces is the same reply the form would have
  made unnecessary — and the request parks until they arrive.
- **It is the honest answer to "the same automatic actions as our factory
  form."** Not *similar* actions. The same ones, because it is the same store.

What this does **not** license: inventing a manifest type from an email. The
manifests are hand-written and reviewed; the classifier's job is recognition
against a fixed list, and an unrecognised email is tagged and left in `/mail`,
never promoted into a type nobody wrote.

---

## 7. Should mail go into the graph?

Raised as an open worry — *that is what flowmail did, but it might bloat the
graph, and a knowledge parser over email may not be a good idea.* Both halves
of that instinct are right, and pkg has already acted on both. The question is
substantially already answered, in the direction of yes-but-narrowly, and the
answer is worth reading before anything here is designed.

### pkg already ingests email

`mecha-graph-core/src/sources/mbox.rs` exists and is documented in
`docs/INTEGRATIONS.md`. Point it at a Gmail Takeout export and it produces
`email.thread` episodes. So the schema, the segmentation and the identity
resolution are settled, and none of it has to be invented here:

- **One episode per thread**, not per message — keyed by the root Message-ID
  from `References`/`In-Reply-To`, falling back to a normalised subject. This
  is the single biggest volume decision and it is already made correctly.
- **Identity is deterministic** — From/To/Cc addresses and display names, no
  model involved. `extract.rs:317` is explicit that an LLM must not be the one
  resolving an address to a person.
- **Bulk mail is dropped at ingest.** `List-Unsubscribe`, `List-Id`, or
  `Precedence: bulk` and the message never enters, because *"newsletters would
  swamp the graph at ~zero value."*

### And it has already built the guards the worry is about

The two things that would make email ruin a knowledge graph were anticipated
and are in the code with the reasoning attached:

- **Email is a weaker interaction signal, and is kept separate rather than
  pooled.** `rollup.rs:3`: *"per-channel recency matters semantically — 'met'
  means calendar or Bee co-presence, not email."* `last_email_at` is its own
  column beside `last_meeting_at` and `last_spoken_at`. Email volume therefore
  cannot masquerade as closeness.
- **Email cannot manufacture social ties.** `precheck.rs:49` —
  `NEVER_AUTO = ["colleague_of", "friend_of", "family_of", "mentors"]` — and
  `migrations.rs:161` says why: *"emailing someone does not make them a
  colleague, a claim about social standing."* Those four predicates can never
  auto-accept; they always go to a human.
- **Conversation-recap predicates are auto-rejected as bloat** —
  `discussed`, `mentioned`, `talked_about`, `shared`, `said`. Which is exactly
  the class an email extractor over-produces. Someone has already watched this
  fail and written the filter.

So "should we write a parser to extract knowledge from email" has an answer:
**no, and mecha must not.** `extract.rs` is the parser, it lives in pkg behind
the precheck, and a second extractor in mecha would duplicate it while
bypassing every guard above.

### What mecha should do instead: the distill pattern, verbatim

`distill.rs` is the precedent and it fits without modification. mecha pushes
**evidence, not belief** — an episode through `kg_upsert` — and pkg's
extractor turns it into candidates that wait in the *user's* review queue.
mecha never asserts a fact about mail. Everything that makes distillation safe
carries over unchanged:

- **Idempotence at both ends** — pkg's `(source, source_id)` key makes a
  re-push an update. `source = "email.thread"`, `source_id` = the thread root
  Message-ID, which is the key `mbox.rs` already uses, so the live path and a
  Takeout backfill of the same thread converge instead of duplicating.
- **Taint is recorded, never laundered.** Mail is untrusted by construction,
  so a mail episode carries its taint snapshot on `meta` exactly as a session
  episode does — and, by the same rule, **never carries `corrections`**. A
  supersede-and-demote driven by text from an email is precisely the attack
  `distill::corrections_for` withholds against.
- **Sensitivity is not the default.** `kg_upsert` takes a tier; mail is at
  least `private`, not `personal`.

### The bloat risk is real, and it is the review queue, not the disk

The volume estimate that matters: a working academic inbox is perhaps 30–60
non-bulk *threads* a day after `mbox.rs`'s filter, against roughly five
calendar events. Ten times the episode rate. Disk is irrelevant at that scale
and the rollup genuinely improves — *when did I last hear from this person*
becomes answerable, which it currently is not.

What does not scale is **candidates**. If each thread yields two, that is
60–120 review items a day, and a review queue nobody can finish is a review
queue that stops being read — which quietly disables the whole
human-in-the-loop story pkg is built on.

Which is where this feature has something to offer pkg rather than the other
way round. **The triage classifier is a strictly better filter than the
bulk-mail heuristic, and it runs anyway.** `bucket: ignore` already names the
mail with nothing in it, using the actual content rather than a header
convention that catches newsletters and misses automated notifications. So the
live path can push only `respond` and `notify` threads and get most of the
value at a fraction of the volume, for free.

Two dials worth having from the start, because retrofitting them means
re-ingesting: **which buckets push** (default `respond` + `notify`), and
**whether extraction runs at all or the episode is evidence-only**. Starting
evidence-only is defensible — the rollup, `kg_entity`'s recency, and
`kg_search`'s evidence scope all work without a single extracted fact, and
extraction can be switched on later against a corpus that already exists.

### One stale fact to fix

`docs/INTEGRATIONS.md:256` still says: *"Live sync remains FlowMail's job on
macOS (it holds the Gmail/Outlook OAuth, spec §3); this path is for corpus
backfill without new credentials."*

That owner no longer exists. mecha-mail holds the OAuth now, for both
providers, across every account in `~/.mecha/mail/`. So there is a documented
seam with a dead owner on the other side of it, and this feature is the
natural inheritor — which is an argument for doing it, and an argument for
updating that line either way.

---

## 8. The correction loop

Corrections arrive from three places, and they should stay distinguishable —
the `"Blocked by a hook:"` lesson is that machine state read as human
correction poisons learning.

1. **A reclassification** (`!`, or changing a tag with `g`). The strongest
   signal, and the cheapest: it is a typed before/after pair with no prose. It
   should feed the classifier's few-shot pool directly and a `triage`-domain
   reflection secondarily.
2. **A rejected or edited draft.** Already handled — `diff(args_before, args)`
   on a sent-with-edits outbox item mines a `writing` reflection.
3. **A dismissed task.** flowmail's finding was that dismissing *with a reason*
   is what stopped over-generation. `kg_task_*` would need to carry the reason
   for this to work, which is a pkg-side question.

The thing to be careful about: a classifier few-shot pool is not the same
object as a learned rule, and it must not become one silently. A learned rule
rides in every future run's cached prefix and is provenance-gated for that
reason. A few-shot example injected into a tool-less classifier that emits a
fixed schema is a much smaller blast radius. Keeping them separate is what
lets the classifier learn fast from cheap corrections without widening the
path that `learning.rs` deliberately narrowed.

---

## 9. Skills — the mechanism mecha does not have

Raised while this was being written, and it belongs here because the email
categories in §6 are the clearest case for it: *handle a rec letter request*,
*process an expense receipt*, *decline a speaking invitation politely* are
procedures, not rules, and there is currently nowhere to put a procedure.

**mecha has no skills concept.** The `.claude/skills/` directories in this
repository are Claude Code's, for working *on* mecha; the agent cannot see
them. What exists instead, in descending order of closeness:

| Mechanism | What it is | Why it is not a skill |
|---|---|---|
| `[[subagent]]` profiles | name, description, tool allowlist, `system_prompt`, `max_turns`, model/provider override, `trusted_output` + `answer_shape`; exposed to the parent **as a tool** | closest by far — but config-only, instructions are one inline string, no bundled files, and invoking one always spawns a child agent with its own conversation and turn budget |
| Learned rules | instructions that ride in every run's cached prefix | always-on and global, hard-capped, provenance-gated. Not selectable, and deliberately so |
| Triggers | a stored prompt with a tool allowlist and a workspace | fired by cron, never chosen by the model |
| Hooks | commands at lifecycle points | policy, not capability |

So the gap is real. A subagent profile is *functionally* a skill — a named,
described, narrowly-scoped capability the model chooses to invoke — and its
`description` field already carries the right instruction ("say when to use
it, not just what it is"). What it lacks is the three things that make skills
pleasant: **a body you write as prose in a file** rather than a TOML string, a
place to put **bundled resources** (a letter template, the finance address, an
example of a good decline), and the option to **load instructions into the
current run** instead of always spawning a child.

flowmail already reached for the same thing and got the shape right: prompt
templates as editable TOML in `src-tauri/src/prompts/`, *loaded at runtime, not
compiled in*, with `{{variable}}` interpolation. That instinct — the procedure
is data a human edits, not code — is the one to carry over.

### The constraint that decides the design

A skill's body becomes **trusted instructions inside a run**. That is the same
half-life problem `learning.rs` exists to manage: a learned rule rides in every
future prompt's cached prefix, which is why every reflection carries an
`Origin` and `mecha learn` excludes non-clean ones structurally, before a
prompt is built. A skill is worse, because a rule is one line and a skill is a
page.

Three rules follow, and they are not negotiable:

- **User-authored only.** A skill is never written by a model, never derived
  from a session, and never proposed by `reflect`. If the agent could author a
  procedure that a later run obeys, the provenance gate has been routed
  around by a mechanism that did not exist when it was written.
- **Global store, never the project layer.** `~/.mecha/skills/`, on exactly
  the triggers rule: `[[hook]]`, `[[mcp]]` and `[[subagent]]` are declarable
  in a project's `mecha.toml`, which is a file that arrives with a cloned
  repository. A repo that could ship a skill has been handed a page of
  instructions in your assistant's head.
- **The description is in the prefix; the body is not.** Progressive
  disclosure keeps the cached prefix small and keeps every unrelated skill out
  of context. Loading a body mid-run appends, which the append-only transcript
  handles for free.

### The sketch

```
~/.mecha/skills/<name>/SKILL.md      frontmatter: name, description,
                                      optional tools allowlist
                                      body: the procedure, as prose
~/.mecha/skills/<name>/*             templates, examples, reference data
```

`mecha skills` lists them and prints which are active, in the shape `mecha
tools` already has. A `skill` tool loads one by name into the current run.
`mecha.toml` may *enable or disable* skills by name — a list of strings, no
inline bodies — so a project can narrow the set without being able to author
one.

Two questions this leaves open, both real: whether a skill can name a tool
allowlist that *narrows* the run while it is loaded (attractive, and it is the
`Capabilities` question again — narrowing must be the only direction), and
whether the subagent profile should be reimplemented on top of skills or left
alone. Leaning: leave subagents alone. They are the *delegate* shape, and
skills are the *instruct* shape; collapsing them would lose the fresh
`Conversation` that makes delegation a clean taint boundary.

This deserves its own treatment rather than being decided inside a mail
feature — **`docs/SKILLS-RESEARCH.md` is that survey**, and it supersedes this
section on every point of detail. The mail categories are the forcing
function, and the mail design should assume skills exist:
`handle-rec-letter`, `expense-forward` and `decline-speaking` are three
skills, not three hard-coded branches in a triage run.

Two findings from that survey are worth carrying back here. `SKILL.md` is now
a cross-vendor standard with roughly forty implementations, so the format is
settled and mecha should adopt it rather than invent one. And the security
record is genuinely bad — Snyk scanned 3,984 published skills and found 36.8%
carrying a security flaw and 76 confirmed malicious payloads — which is why
the rules above are not paranoia, and why mecha should ship the format with
**no install path at all**.

---

## 10. Open questions

1. **When does classification run?** Nightly on a trigger is cheap and makes
   `/mail` instant, but stale by afternoon. Live on open is fresh but makes
   opening the modal a model call. Leaning: a trigger classifies, and opening
   `/mail` classifies anything unseen — so the common case is a store read and
   the uncommon case is honest about waiting.
2. **Which model classifies?** flowmail used Haiku. mecha's thesis is the
   local open-weight model, and triage is close to the ideal job for it: short
   input, typed output, high volume, low stakes, and a correction loop that
   measures whether it is working. This is a demonstration of the thesis, not
   a compromise on it.
3. **Bodies or snippets?** Full bodies classify better and cost far more on a
   local model. Probably snippet-first with an escalation rule, and the
   escalation rule is itself measurable once the store exists.
4. **Archive under `permission_mode = "read-only"` — refuse or allow?**
   Leaning refuse. An unattended run that empties the inbox is not what
   read-only promises, and the useful unattended shape — *draft my replies
   overnight* — already works without it, because staging executes nothing.
5. **Is there an autonomy tier, ever?** Inbox Zero's per-rule graduation is the
   good version: one rule promoted from propose to execute at a time, never a
   global switch. It maps onto the existing proposal gate. But it is v2 at the
   earliest, and it should follow `/review now|later|auto`'s rule — set only by
   explicit command, never inferred, because a policy that decides what leaves
   the machine must not be decidable by anything sharing a context window with
   third-party text.
6. **Does the triage store hold bodies?** If it does, it is a second copy of
   the mailbox under `~/.mecha` with its own retention question. If it does
   not, `show` needs a live fetch. Leaning: ids and the verdict only, fetch on
   demand — the store is an index, not a cache.
7. **Is `Mail.ReadWrite` worth an IT ticket?** The only genuinely blocked
   item (§5). The alternatives are Gmail-only triage verbs, or a personal
   Microsoft app registration for the Dartmouth account — which trades the
   admin-consent problem for a separate-registration problem and may violate
   policy anyway. This is a question for the user and their IT, not a design
   question, and it should be asked early because it decides whether phase 1
   ships to one account or two.

---

## 11. What not to build

- **Cards.** §3. Three owners already exist for the card's three jobs.
- **A second approval surface.** The outbox is it. `/mail` proposes; `/outbox`
  releases.
- **`mail_snooze`.** Neither provider has one — Gmail's is client-side. It
  would have to be a label plus a trigger that removes it, and a snooze that
  silently means *labelled and forgotten* is the silently-degrading-sandbox
  shape in a new costume.
- **An open tag vocabulary.** It drifts into synonyms and stops filtering.
- **A manifest type invented from an email.** §6.
- **Auto-send of anything, at any confidence.** Including the finance forward,
  which is the most tempting one precisely because it feels mechanical.
- **A mail cache or a local index.** Search fans out to the providers already,
  and the triage store is an index of verdicts, not of mail.

---

## 12. A phase plan

Ordered so each phase is useful alone and testable without the next.

**Phase 1 — the verbs.** `mail_archive`, `mail_mark_read`, `mail_spam`,
`mail_trash`, `mail_tag`, plus `unread_only` and the read-state fields on
reads. Capability labelling per §5, with a `assert_tool_surface` test naming
the new quadrant — nothing in `[outbox] tools`, everything `destructive`.
Testable with no model at all, and it unblocks everything else regardless of
how the UX questions land.

**Phase 2 — the classifier and the store.** `~/.mecha/mail-triage/`, the
tool-less extractor, a `mecha mail classify` verb, and a trigger that runs it.
No UI. At the end of this phase the morning briefing can already read typed
verdicts instead of re-reading the inbox, which is a real improvement on its
own.

**Phase 3 — the CLI.** `mecha mail list | show | archive | spam | tag | task |
reply`. The front door's rule holds: the command line does everything first,
and the modal drives the CLI. `show` prints prose for a human; `list` prints
typed fields only.

**Phase 4 — front-door routing.** `request_type` recognition against the
manifest list, and promotion of a recognised email into `~/.mecha/requests/`.
This is where the letter and lab-application cases get their real behaviour,
and it is worth doing before the TUI because it changes what the TUI has to
show.

**Phase 5 — `/mail`.** The modal, on the `/outbox` pattern.

**Phase 6 — the correction loop.** `!`, the few-shot pool, and a `triage`
domain in the learning store, kept distinct from learned rules per §8.

**In parallel, on its own track — skills (§9).** Not a phase here, because it
is not a mail feature and should not be designed inside one. But phases 4 and
6 are much better with it: the per-category behaviour wants to be
`handle-rec-letter` and `expense-forward` as editable files, not branches in a
triage prompt. If skills land first, the mail design gets simpler. If they do
not, phase 4 hard-codes the four manifest types and the rest waits.

---

## Sources

Surveyed 2026-08-17.

- [langchain-ai/agent-inbox](https://github.com/langchain-ai/agent-inbox) —
  the four-action enum and its per-item config
- [langchain-ai/executive-ai-assistant](https://github.com/langchain-ai/executive-ai-assistant)
  and [agents-from-scratch architecture](https://deepwiki.com/langchain-ai/agents-from-scratch/2-email-assistant-core-architecture)
  — IGNORE/NOTIFY/RESPOND, and the three memory kinds
- [elie222/inbox-zero](https://github.com/elie222/inbox-zero) ·
  [openalternative writeup](https://openalternative.co/inboxzero) —
  plain-English rules and per-rule graduation to autonomy
- [AgentMail — agent inbox, and how to design one safely](https://ai.agentmail.to/agent-inbox-what-it-is-when-to-use-it-and-how-to-design-one-safely)
- [Nylas — human-in-the-loop email agent](https://cli.nylas.com/guides/build-human-in-loop-email-agent) ·
  [AI email triage agent](https://cli.nylas.com/guides/build-ai-email-triage-agent)
- [Drafts as a human approval gate for agent email](https://dev.to/qasim157/drafts-as-a-human-approval-gate-for-agent-email-308k)
  — the stale-approval problem, which the outbox's recorded jail is the
  analogue of
- [Superhuman vs Shortwave](https://cmdk.email/post/superhuman-vs-shortwave/) ·
  [Zapier's comparison](https://zapier.com/blog/shortwave-vs-superhuman/)
- [aerc](https://aerc-mail.org/) · [pimalaya/himalaya](https://github.com/pimalaya/himalaya) ·
  [Best email clients for developers, 2026](https://email-tools.me/posts/best-email-clients-developers/)
- [haasonsaas/email-agent](https://github.com/haasonsaas/email-agent) — a TUI
  email agent with multi-agent categorisation
- Local: `~/Github/flowmail` — `CLAUDE.md`, `dev_docs/AI_PIPELINE.md`,
  `dev_docs/DATA_MODEL.md`, `src/components/{triage,focus}/`
