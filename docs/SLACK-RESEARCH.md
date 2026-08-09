# Slack as a remote control — research

**2026-08-09.** One question: *how should mecha be driven from Slack, so that a
phone is a real control surface for the agent — start work, watch it, answer its
questions, move files both ways — without weakening anything the harness
enforces?*

Researched by reading Slack's platform documentation directly
(`docs.slack.dev`, which now serves what `api.slack.com` redirects to), the
published documentation for Claude's own remote surfaces, and this repository.
Every numeric limit below came off a live method-reference page on this date;
anything that could not be pinned to a primary source is marked **UNVERIFIED**
and should be re-checked before it is built against. The Slack AI surface moved
substantially in the first half of 2026, so treat this document as perishable in
a way most of the others here are not.

[`HANDOFF.md`](HANDOFF.md) has carried "Slack as a transport — zero lines exist;
**the blocking decision is the identity model, not the socket**" for some time.
§3 answers that decision. The rest is what follows from it.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| How does Slack reach a workstation behind NAT? | **Socket Mode.** mecha dials Slack; no inbound port, no certificate, no signature verification | §2 |
| Who is allowed to drive it? | **An allowlist of Slack user IDs, bound once by a nonce the local CLI prints.** Everyone else is the front door, not a lesser user | §3 |
| What is the unit of work? | **A Slack thread is a `Conversation`** — which gives the trifecta interlock the right granularity for free | §4 |
| How is a long run reported? | `chat.startStream` + `task_update` chunks, one per tool call; `assistant.threads.setStatus` for the gap before the first token | §5 |
| How are approvals handled? | **The outbox is the primary surface, not a live prompt.** The human is away by definition; staging is what mecha already does about that | §6 |
| Files and images? | Out: private upload + `slack_file` reference. In: **download into the run's workspace and name the path in the prompt** — no core type change in phase 1 | §7 |
| Should the factory carry this? | **No — the box must never be an owner channel.** It carries artifacts; Slack carries control | §8 |
| Is this a big build? | The transport is small. The two costs are the identity binding and the approval-timeout semantics | §10 |

The one-sentence version: **Slack is the TUI's message-passing front-end with a
WebSocket where the terminal was, and an absent human where the present one
was** — and every design decision below falls out of that second clause.

---

## 1. What is actually being asked for

Four capabilities, and they are not equally hard:

1. **Run anything** — start a run from a phone. *Easy*: this is a prompt and a
   `RunContext`.
2. **Monitor** — watch it happen. *Easy now, hard a year ago*: Slack shipped a
   real streaming API in October 2025 (§5), so this is no longer a `chat.update`
   loop fighting a rate limit.
3. **Answer questions** — the agent asks, the human answers. *Medium*: the
   `Approver` and `Asker` traits are already `async`, so a remote human is
   expressible. What is hard is what happens when nobody answers (§6).
4. **Pass files and images both ways** — *the only one that reaches into
   `mecha-core`*, and only in one direction: mecha is text-only end to end
   (§7).

Everything else — steering a run in flight, cancelling, switching modes,
reviewing drafts — already exists behind the CLI and the TUI, and needs a
transport rather than a feature.

---

## 2. The transport: Socket Mode

### What Slack offers

Two ways to receive events:

| | **Socket Mode** | **HTTP Events API** |
|---|---|---|
| Direction | mecha dials Slack (`wss://`) | Slack dials a public URL |
| Opened by | `apps.connections.open` with an app-level token (`xapp-`, scope `connections:write`), Tier 3 | — |
| Inbound requirements | none | public HTTPS, valid certificate, `url_verification` challenge answered in 3 s |
| Request signing | **none needed** — the socket is pre-authenticated by the token that opened it | `v0:{timestamp}:{raw body}` HMAC-SHA256, constant-time compare, reject skew > 5 min |
| Ack | echo `envelope_id` over the socket | HTTP 2xx within 3 s |
| Retries | UNVERIFIED whether unacked envelopes replay | exactly 3 — immediate, +1 min, +5 min |
| Failure cliff | reconnect | **>95% delivery failure in a 60-min window disables your event subscriptions** |
| Concurrency | 10 connections per app | n/a |
| Reconnect | every few hours; `refresh_requested` gives ~10 s warning | n/a |
| Restriction | **cannot be listed in the Slack Marketplace** | none |

Slack's own guidance recommends HTTP "for production applications" because
Socket Mode is stateful and harder to scale horizontally. That advice is aimed
at multi-tenant SaaS serving thousands of workspaces. It is the opposite of the
right advice for one workstation serving one person.

### What this means for mecha

**Socket Mode, one connection, and it is not close.** It deletes, in order: the
NAT problem, the dynamic-DNS problem, the TLS certificate problem, the tunnel
process to babysit, the raw-body-handling and timing-safe-comparison code that
request signing demands, and the 95%-failure auto-disable cliff that a
workstation reboot would otherwise walk into. The Marketplace prohibition costs
nothing for a personal tool.

**And the argument is already in this tree.** `scripts/mecha-drain.service` was
written for a different problem and reached the same shape:

> *"home holds the connection open so nothing ever has to dial home, and
> 'instant' is a property of who waits, not of who calls."*

Socket Mode is that sentence applied to Slack. The precedent also carries the
operational form: an always-on `systemd --user` unit with
`Restart=always`, `Environment=PATH=%h/.cargo/bin:…` (the omission of which has
bitten this project four times), running from `~/.cargo/bin` rather than a repo
checkout — because a shared checkout sits on whatever branch someone is working
on.

Two costs to own deliberately:

- **Reconnect is yours.** Honour `refresh_requested` by opening the new socket
  before closing the old, so no window exists where an event has nowhere to
  land.
- **Assume no replay.** Whether Slack redelivers unacked envelopes across a
  mid-session drop is UNVERIFIED, so make every handler idempotent on
  `event_id`, and accept that messages arriving while the connector is down are
  lost. A `mecha slack catch-up` that reads `conversations.replies` since the
  last seen `ts` closes that hole cheaply — and the 2025 throttle on that method
  does not apply to us (§3).

**New dependency:** a WebSocket client. `mecha-core` has `reqwest` 0.12
(rustls, streaming) and `tokio`, but nothing that speaks WebSocket, and
`tokio`'s `net` feature is not even enabled. `tokio-tungstenite` with
`rustls-tls-webpki-roots` is the boring choice and matches the existing TLS
stack.

---

## 3. The identity model — the decision that was blocking

Slack is a multi-party medium. mecha has exactly one principal: its owner.
Reconciling those is the whole problem, and the mistake available here is to
invent a middle tier.

### The two tiers, and why there is no third

**Owner.** A pinned allowlist of Slack user IDs in local config. Checked against
`payload.user.id` on every inbound interaction — **before** the message reaches
a prompt, before the approver, before any dispatch. An owner drives the agent:
starts runs, steers, cancels, approves, releases drafts, changes the mode.

**Everyone else is the front door.** This is the load-bearing idea, and it means
no new trust tier has to be designed: mecha already has a component whose entire
job is *"a stranger wants something from a run that holds the calendar and the
mailbox"* — `frontdoor.rs`, and the sentence it serves is

> **The privileged run sees the extraction, never the prose.**

A Slack message from a non-owner is a stranger's request that happens to have
arrived over a WebSocket instead of through `mecha-factory-publish drain`. It
should become a front-door record, be extracted by a pass with no tools and no
history, and reach a privileged run only as typed values. Nothing new is
invented; the quarantine that already exists simply gains a second inlet.

There is deliberately **no** "trusted colleague who may run read-only commands"
tier. It is the tier everyone wants and nobody can measure: read-only still
reads the mailbox, and the interlock's whole premise is that a human clicking
yes is what an injection is trying to engineer. Two tiers is a boundary; three
is a policy, and a policy needs evidence this project does not have.

### Binding the owner, once

Slack gives you no ownership primitive. Email is not proof — `users:read.email`
returns the *workspace's claim* about an address, not evidence the person
controls it, and workspace admins can change it.

The construction that holds:

1. `mecha slack link` prints a short-lived nonce **on the workstation**.
2. The user types it into a Slack DM or a modal.
3. The connector binds that `user.id` and stores it.

That proves the Slack user has shell access to the machine the agent runs on,
which is exactly the claim that matters and the only one worth having. Store
`U…` IDs, not emails — emails change, IDs do not. Bind `team_id` (and
`enterprise_id`) too, so a distributed install cannot deliver events from
somewhere else.

**The allowlist is security-critical config that lives beside the tokens.** It
must not be reachable by a tool, proposable by the learning system, or settable
by anything that shares a context window with third-party text. This is the same
rule the TUI already applies to `/review now|later|auto` — *"set only by slash
command, never inferred from the prompt, because release policy must not be
decidable by anything sharing a context window with third-party text"* — and it
generalises here without amendment.

### Which kind of Slack app

**An undistributed, single-workspace internal app.** One `xoxb-` bot token, one
`xapp-` app-level token, no OAuth callback to host, no installation database, no
`state` parameter, no signing secret (Socket Mode needs none).

That choice is worth more than the convenience, because of a 2025 platform
change worth knowing about: from 2025-05-29 Slack restricted
`conversations.history` and `conversations.replies` to **1 request/minute and 15
objects per request** — with the `limit` collapse silent, so you ask for 100 and
get 15 and no error. The trigger for that restriction is **commercial
distribution outside the Marketplace**, and Slack published a follow-up
changelog on 2025-06-03 for the sole purpose of saying *"any internal
customer-built apps will maintain their existing rate limits."* An internal app
keeps Tier 3 and 1,000-object pages — which is what makes the catch-up path in
§2 viable — and is also the category eligible for Slack's Real-Time Search API
and Slack's own hosted MCP server, both of which exclude unlisted distributed
apps.

Note the trap in the other direction, in case distribution is ever
reconsidered: **a Socket Mode app cannot be Marketplace-listed**, so a Socket
Mode app that is commercially distributed has no escape route from the
restricted limits.

**Skip token rotation.** It is opt-in and *irreversible* ("may not be turned off
once it's turned on"), expires access tokens every 12 hours, permits at most 2
active tokens per installation, and demands a locked refresh path. It buys a
single-user internal tool nothing. (If it is ever turned on, refresh behind a
lock — `mecha-mail`'s OAuth lifecycle already has the shape, and the reason it
has it is that two concurrent tool calls raced two refreshes.)

### Scopes

Minimum for everything in this document:

`chat:write`, `assistant:write`, `im:history`, `app_mentions:read`,
`files:read`, `files:write`, `users:read`, `users:read.email` (only for the
one-time lookup at link time), `commands` — plus app-level `connections:write`.

`users:read` alone no longer returns the email field; `users:read.email` is
separately required.

---

## 4. The unit of work: a thread is a conversation

**One Slack thread ↔ one `agent::Conversation` ↔ one session ↔ one workspace.**

This is not an aesthetic choice. Taint lives on `Conversation` alongside the
messages, precisely so that *"keep the history and you keep the taint; start a
new `Conversation` and you get a clean one."* Mapping a thread onto a
`Conversation` therefore gives the interlock exactly the right granularity for
free: a new thread is an honest clean slate, and a thread that fetched a hostile
page on Monday still remembers on Tuesday. Any other mapping — one conversation
per channel, one per day, one global — has to re-answer a question that is
already answered correctly.

Consequences that follow:

- **Each thread gets its own workspace**, `~/.mecha/work/slack-<thread_ts>/`,
  exactly as a trigger gets one per producer. `work::ensure_outside_mecha_home`
  already refuses a jail that contains `~/.mecha`, and the retention policy
  (`[work] keep`, swept nightly) already applies. Nothing new.
- **Concurrent threads are concurrent runs**, which the core was built for:
  `Agent::run_in` takes a caller's `RunContext` so that *"one agent — one
  provider connection, one cached prefix — serves concurrent runs jailed to
  different directories under different permissions."* The connector is one
  process holding one `Agent`, spawning one task per active thread.
- **One process owns a session.** `session.rs` appends with no lock, no fsync
  and no exclusion — the model is read-at-start, append-at-end, with the
  in-memory `Conversation` as the truth. Two writers would interleave records
  and each would resume from a transcript the other is mutating. So the
  connector must own every session it drives, and must refuse to attach to one a
  TUI has open. The `.running` marker pattern from the trigger store is the
  precedent — including the detail that cost something there: **a marker whose
  pid is gone reads as *not* running**, with a range check on the pid, because
  `kill(-1, 0)` succeeds and would report every dead run as alive.

### The thread-context gotcha

Slack's assistant surface delivers user messages as plain `message.im` events
that **carry no thread context**. Only `app_context_changed` does. Bolt hides
this behind a thread-context store; a hand-rolled loop must persist context
keyed on `(channel_id, thread_ts)` or lose it silently. Budget for it.

Related and more important: the surface itself was restructured on 2026-06-30.
`assistant_view` became `agent_view`; agent threads now live in the ordinary
Messages tab rather than a split pane; **`assistant_thread_started` is no longer
the entry point** (use `app_home_opened` with `tab="messages"`); and
`assistant_thread_context_changed` became `app_context_changed`. Verified
directly against the changelog: *"New apps can only use the Agent messaging
experience."* The migration is one-way, so building against the legacy names
would be building against something already closed.

---

## 5. Monitoring: Slack has a real streaming API now

This is the section most likely to be stale in anyone's memory, so it is worth
stating plainly: **do not build progress reporting out of `chat.update`
loops.**

| Method | Scope | Tier | Notes |
|---|---|---|---|
| `chat.startStream` | `chat:write` | Tier 2 (20+/min) | requires `channel` + `thread_ts` |
| `chat.appendStream` | `chat:write` | **Tier 4 (100+/min)** | `markdown_text` ≤ 12,000 chars/call |
| `chat.stopStream` | `chat:write` | Tier 2 | may carry final blocks |

Verified live on this date. Chunk types: `markdown_text`, **`task_update`**,
`plan_update`, `blocks` (≤50 per array). `task_update` and `plan_update` cap at
256 characters each. `chat.startStream` takes `task_display_mode` ∈ `timeline`
(default) | `plan` | `dense`.

The rate-limit *shape* is the tell: start and stop are Tier 2 while append is
Tier 4, which is Slack saying "many appends per stream" out loud.

There is also a `task_card` block — `task_id`, `title`, `details`, `output`,
`sources[]`, and `status` ∈ `pending | in_progress | complete | error` — which
is a tool call's lifecycle with the names already chosen.

### The mapping

`AgentEvent` is the seam (`mecha-core/src/agent.rs`), and it maps almost
one-to-one:

| `AgentEvent` | Slack |
|---|---|
| *(before the first token)* | `assistant.threads.setStatus` |
| `TextDelta` | buffered `markdown_text` chunks |
| `ThinkingDelta` | **nothing** — see below |
| `ToolCall` | `task_update` → `in_progress` |
| `ToolResult` | `task_update` → `complete` / `error` |
| `ToolDenied` | `task_update` → `error`, **naming which layer denied** — interlock, hook, approver, or timeout (§9's "surface denials where the human already is") |
| `Compacted` | a `task_update` note — a summary was taken |
| `QueuedInput` | echo, so steering is visibly received |
| `Done` | `chat.stopStream` with a usage/cost footer |

Four details worth writing down:

- **`setStatus` is 600/min and auto-clears after two minutes with no message
  sent.** That auto-clear is a feature, not a limitation: a connector that dies
  mid-run cannot leave a spinner running forever. It also writes only to the
  requesting user's own view, which is why it is **not** a send sink (§6).
- **Buffer by size, not by token**, and self-throttle to roughly one append per
  second. That is Slack's own general design rule and it costs nothing here.
- **Do not stream thinking.** A Slack thread is a medium other people can read,
  and thinking blocks are the least reviewed text the model produces. The TUI
  can show them because the terminal has one reader.
- **Always send `stopStream`.** A dropped frame must never be the last thing the
  user sees; the terminal call is what distinguishes "finished" from "died".

`AgentEvent` derives only `Debug, Clone` — it is not `Serialize`. That is
correct and should stay that way: mapping to a wire format is a front-end
concern, and keeping the connector **in-process with the agent** means no wire
format is needed at all. This is a real argument for the connector being a mecha
subcommand rather than a separate daemon talking to mecha over something.

---

## 6. Approvals — and why the outbox is the primary surface

### What Slack makes possible

`Approver::approve` and `Asker::ask` are both `#[async_trait]`, so a remote
human is expressible today with no core change. `TuiApprover` is the working
proof: it sends `Request { tool, summary, reply: oneshot::Sender<Answer> }` over
an mpsc and awaits the oneshot. A `SlackApprover` is the same type with a Slack
message where the modal was.

Mechanically, the right Slack shape is a **durable posted message, not an
ephemeral one**: ephemerals "do not persist across reloads, desktop and mobile
apps, or sessions", delivery "is not guaranteed — the user must be currently
active in Slack", and `chat.update` cannot touch them. For a remote control
whose entire premise is that the human is elsewhere, that is disqualifying. Post
a `card` (≤3 buttons) or a section plus an `actions` block, then `chat.update`
it into a terminal "approved by @x at T" state so the record is permanent and
the buttons cannot be re-clicked.

Two rules that are not obvious:

- **Gate on `payload.user.id`, never on the button's `value`.** The `value`
  field carries a correlation id and nothing else; anything authorising can be
  influenced by whatever composed the message.
- **`trigger_id` expires in three seconds and is single-use.** If a rejection
  should open a modal to collect a reason, open it *first* and do the work
  after.

### Why it should nonetheless be the exception

Everything above describes a mechanism this design should use sparingly, because
of what the async signature hides: **the agent turn blocks while the approval is
awaited.** There is no "approve later, continue now" path in the loop. A run
that asks a question at 2am is a run that is stopped until morning.

mecha already has the right answer to that, and it is not a faster prompt. It is
the **outbox**: nothing leaves the machine at stage time, the model is told it
staged a draft and keeps going, and review happens out of band in another
process, hours later. That is precisely the shape a remote control needs, and it
was designed for a human who is away.

So the recommendation is a posture, not a feature:

> **A Slack-driven run defaults to the trigger's posture — read-only unless the
> owner says otherwise, everything outbound routed to the outbox — because the
> human is away by definition.**

Which makes the primary Slack review surface the outbox, not a live modal. And
that closes a deferral recorded in
[`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) §11: *"A phone UI for
releasing outbox drafts — the best argument for a home-side server, and a
separate project."* Slack is that phone UI, and it needs no home-side server at
all. The scoping rules the TUI already worked out transfer unchanged: an id-diff
between submit and completion so no mode touches items another session staged,
tainted drafts never auto-released, an errored run releasing nothing.

### The trap: a timeout must not say "Denied by the user"

This is the finding in this section that would otherwise be discovered
expensively.

Everywhere in mecha, **silence is never approval**: `TerminalApprover` treats
EOF as "n", `TuiApprover` denies on a closed channel and on a dropped sender,
`ModeApprover` in `Ask` mode denies because nothing is watching to answer, and
`spawn_detached` gives its children a null stdin *because* EOF means no. A Slack
approval that nobody answers must therefore deny. That much is settled.

But a denial is not a neutral event here, because `CLAUDE.md` records that the
learning miner **keys on the exact string `"Denied by the user:"`**, and that a
hook denial deliberately reads `"Blocked by a hook:"` instead — *"machine policy
is not a user correction, and learning from it would teach mecha rules it was
already obeying."*

An approval that timed out because the owner was asleep is machine policy
wearing a human's clothes. If it returns `Deny("Denied by the user: …")`, then
every unanswered 2am prompt becomes training data, and mecha learns rules from a
human who was not there. That is the same mistake as mining a publish's changed
path as a voice correction, one costume further on.

So: **a timeout denial needs its own string** — something like `"No answer from
Slack within 10m:"` — and it must be excluded from the miner, with a test named
on it, exactly as the hook string has one.

Two more inherited details:

- **`Answer::Always` must not exist in Slack**, or must be scoped to the thread.
  It is remembered in a process-local `Mutex<HashSet<String>>` and never
  persisted, which is fine for a TUI session that ends when the terminal closes
  and is a much larger blast radius in a connector that runs for months.
- **`ask_user` can be registered**, since a Slack connector does own a human —
  asynchronously. Its timeout returns `None`, meaning declined, and the decline
  wording is load-bearing and already measured: an A/B on this machine found
  *"proceed with your best interpretation"* made the model invent. Reuse the
  existing wording verbatim; do not paraphrase it for Slack.

### Elevating, when "run anything" is genuinely wanted

The default posture above is not a ceiling. A per-thread mode change
(`/mecha mode write`, say) is fine, subject to three rules, all of which already
have precedent: only an allowlisted owner may set it; it is set by an explicit
command and **never inferred from prompt text**; and it is per-thread, so
elevating one piece of work does not elevate the next.

---

## 7. Files and images, both directions

### Outbound — easy, with one boundary decision

`files.upload` is retired (announced 2024-04-09, blocked for new apps
2024-05-16, sunset 2025-11-12; Slack never published a completion changelog and
the reference page is still future-tense, so runtime behaviour today is
UNVERIFIED — plan for it being dead). The current flow is
`files.getUploadURLExternal` → POST the bytes as `application/octet-stream` →
`files.completeUploadExternal`. Both API calls are Tier 4; the limit is 1 GB per
file on all plans, 1 MB for snippets.

The pattern worth using: **upload with no `channel_id`, so the file stays
private, then reference it from an `image` block via `slack_file: {id}`.** A
rendered chart or a diff image appears inline in the thread without ever
becoming a public URL. The documented footgun is that the upload and the post
must use the same token, or the app cannot display its own file.

For code and logs, `rich_text_preformatted` now takes a `language` field for
syntax highlighting and has no documented character cap — better than a mrkdwn
section, which is bound by the section block's hard 3,000. Above ~1 MB, or when
the artifact is a directory, publish it (§8) and post the link.

**The boundary decision.** Posting to Slack is sending, and the trifecta
interlock exists because a run holding private data and untrusted content must
not send. But replying to the owner's own DM is not exfiltration — it is the
terminal printing the answer, with a longer wire. So the line is:

- **The reply address is set by the transport, never by the model.** There is
  deliberately no `slack_post(channel, text)` tool. The connector answers in the
  thread that invoked it, and there is no argument that makes it answer
  elsewhere. This is the `Record::for_privileged_run` pattern — *"the boundary
  is a function, not a rule"* — applied to an address instead of to prose.
- **A DM reply is not a send sink.** The recipient is the principal.
- **A channel reply is**, because other people read it. A tainted run that was
  invoked from a channel should answer in the owner's DM instead of the channel,
  or stage.
- If a general `slack_send` tool is ever built, it is `external_send`, it is
  named in `[outbox] tools`, and it is a different feature from this one.

And one Slack-specific exfiltration channel that the interlock would otherwise
never see: **set `unfurl_links: false` and `unfurl_media: false` on everything
the model authors.** Slack's own security guidance names unfurling as the step
that "issues the immediate, unauthorized HTTP request that would complete the
data exfiltration" — a model-emitted URL becomes an outbound GET with no tool
call anywhere. It is exactly why `http_fetch` is a send sink despite being
read-only, and it is free to prevent.

### Inbound — the only part that reaches into `mecha-core`

Two sub-problems, and they should be sequenced rather than solved together.

**The type problem.** `message.rs` has four block kinds — `Text`, `Thinking`,
`ToolUse`, `ToolResult` — and no image or binary variant anywhere. `ToolOutput`
is a `String`. Neither provider encodes anything else. Adding
`Block::Image { media_type, data }` is additive on the wire (the enum is
`#[serde(tag = "type")]`, so old transcripts still load), but it touches the
Anthropic body builder, the OpenAI translator, the streaming reassembler, and —
the part that is an audit rather than an edit — **every consumer of
`Message::text()`, which silently drops non-text blocks today**: compaction, the
learning miners, the eval graders, `RunOutcome.text`. There is also a live
question of whether the local model behind `[providers.local]` accepts images at
all.

**Phase 1 needs none of that.** Download the file into the run's workspace and
name the path in the prompt: *"the user attached `screenshot.png`; it is at
`./inbox/screenshot.png`"*. This is exactly what Claude Code's mobile app does —
attachments are downloaded to the machine and referenced as file paths — and it
works with a text-only model, needs zero core changes, and lets the agent reach
the bytes with the tools it already has. Phase 2 is `Block::Image`, when a
vision-capable provider is the target and the `Message::text()` audit has been
done.

**The security problem**, which applies in both phases:

- Bytes from Slack are third-party content. The download path arms
  `untrusted_input` and its output must be marked `.from_outside()`.
- The download itself has a confirmed silent failure: `url_private` requires
  `Authorization: Bearer <token>` with `files:read`, and **a missing,
  under-privileged or stripped token returns HTTP 200 with `text/html`** — a
  redirect stub or a full sign-in page, not a 401 and not JSON. Because
  `files.slack.com` redirects to `<team>.slack.com` and HTTP clients strip
  `Authorization` across hosts, following redirects reproduces this reliably. So:
  send the header explicitly, **disable redirect following**, **reject
  `text/html` even at HTTP 200**, and cross-check the byte count against the
  file object's `size`. Without those four, a Slack login page ends up in the
  model's context labelled as the user's screenshot.
- Both `file_shared` and `message`-with-`files[]` fire for a single upload, and
  the former carries only a stub. Consume `files[]` off the message event, but
  **register a no-op ack for `file_shared` anyway** — an unhandled subscription
  returns 404 per delivery and Slack retries three times per upload.
- Files land inside the thread's jail, under an `inbox/` subdirectory, and the
  existing retention sweep applies. `frontdoor::Attachment` is the precedent for
  inbound bytes and it is worth reading first: it keeps bytes *beside* the store,
  outside every workspace, and hands the privileged run measurements rather than
  content. That is the stricter posture, appropriate for a stranger; an owner's
  own screenshot can go in the jail.

---

## 8. Where the factory fits, and where it must not

The factory is the obvious place to put a remote control — it is already a
deployed public box with sessions, magic links, scoped credentials, a tenant
model, and, most temptingly, **exactly the right channel already built**:
`GET /v1/queue?since={seq}&wait={s}` is a long-poll that home holds open, and
`mecha-drain.service` is a live always-on loop consuming it. Routing Slack
events through the box would reuse machinery that exists and works.

**It should not, and the reason is worth stating precisely.**

The box's stated invariant is *"the box holds no credential that reaches
home"* — two Argon2id hashes, the published bytes, a certificate — and the
property behind it is that **packets go one way**: mecha publishes and drains,
and the origin never dials home. A Slack integration hosted there breaks both
halves in ways that are easy to under-rate:

- A Slack bot token on the box is a credential that reaches **your workspace**.
  That is a larger prize than everything the box currently holds, and the box is
  explicitly *"assumed lost"* in its own crate description.
- Even preserving the pull direction — home long-polls a command queue, so the
  box still never dials home — changes what a compromised box can do. Today the
  worst it can hand you is a typed request record, which goes through the front
  door quarantine and reaches a privileged run as an extraction. A command queue
  makes the box able to **instruct your agent**. The direction of the packets
  would be preserved while the direction of the *authority* is inverted, which
  is the more important of the two.

The rule that falls out is short: **the box may never be an owner channel.**
Anything arriving from the box is a request, not a command, and goes through
`frontdoor.rs` — which is exactly what happens today and should keep happening.

What the factory *should* do for this feature is the thing it is already good
at: **carry the bytes.** Slack is a poor container for a 40-page report, a
notebook, a rendered dashboard, or a directory of generated files, and it has
hard caps (3,000 chars in a section, 12,000 in a markdown chunk, 50 blocks in a
message). A run that produces something large publishes a bundle and Slack
carries the capability URL. The division of labour is clean and each side plays
to its strength:

> **Slack carries control and conversation. The factory carries artifacts.**

A later, optional third thing: the factory's existing magic-link session
machinery could serve a read-only web view of a session transcript, for the
times a phone needs to scroll a long run. That is additive, it is still
read-only, and it does not make the box an owner channel. It is not needed for
v1.

There is a fourth architecture that Slack now offers and that should be recorded
as **considered and rejected**: the Slackbot MCP Client (June 2026) inverts the
relationship — Slack becomes the MCP *client* and your agent an MCP *server* at
a public HTTPS URL, with Slack performing per-tool user approval natively
(allow-once / always-allow / deny, with `readOnlyHint` tools exempt). The
approval UX is genuinely good and would come for free. It is wrong here for two
reasons: it requires the public endpoint Socket Mode was chosen to avoid, and it
inverts the trust relationship — **Slack's model, not mecha's loop, would be
deciding when to call mecha's `shell`**, with mecha's tool surface exposed to it.
That is a different product: giving Slack's assistant mecha's tools, rather than
giving mecha a remote control.

---

## 9. Prior art

Six systems were read for how they expose a long-running agent to a remote
human: Claude Code's remote surfaces, Claude in Slack ("Claude Tag"), OpenAI's
Codex cloud, GitHub's Copilot cloud agent, and Amp. Claims below come from
vendor documentation on this date; the ones that would change a decision are
flagged where they are secondhand.

### The architecture Claude Code converged on, which is the one recommended here

**Claude Code Remote Control** (`claude remote-control`, or `/remote-control`
in a live session) prints a URL and a QR code; a phone or another browser then
drives the session. The mechanism is the load-bearing part:

> The local process stays running and **polls for work**. Control travels
> through the vendor's API; **the filesystem, MCP servers, and local tools stay
> on the machine.**

That is exactly §2 plus §4 of this document, arrived at independently by the
product whose UX the request names. Details worth copying:

- **Forwarded dialogs expire.** Permission prompts and questions pushed to a
  remote surface time out — five minutes by default, configurable. Confirms that
  §6's timeout is a real requirement and not an edge case, and gives a
  defensible default order of magnitude.
- **Push notifications are separate toggles**, one for "Claude decided
  something" and one for "an action is required". The second is the one that
  matters for an away human.
- **Session continuity survives sleep and network drops**, reconnecting
  automatically, but a machine offline for **more than ten minutes** ends the
  session. A remote control needs a documented liveness rule, not an implicit
  one.
- **Some things stay local-only** — resume, plugins, capture. Not every command
  needs a remote form, and saying which do not is part of the design.
- **Attachments from mobile are downloaded to the machine and referenced as file
  paths.** This is the direct precedent for §7's phase 1, and it is what a
  vendor with full multimodal support chose anyway.

**Claude Tag** (Claude in Slack) is the closest surface-level analogue, and the
useful parts are its differences:

- **Identity switches with the container.** In a channel Claude acts under an
  admin-provisioned *service* identity with channel-scoped access; in a DM it
  switches to the sender's own account and their personal connectors. Two
  identity models in one product, chosen by where you typed. mecha's answer in
  §3 is simpler because mecha has one principal — but the lesson is that
  *channel and DM are genuinely different trust contexts*, which is also where
  §7's send-sink line falls.
- **Progress is a checklist edited in place**, and the documentation explicitly
  warns that Slack does not notify on edits, so *"a quiet thread usually means
  Claude is working, not stuck."* That is an argument for `task_update` chunks
  over edited messages, and for a terminal `stopStream` that makes completion
  unambiguous.
- **Anyone in the channel can steer**, and a colleague's thread is yours to
  continue. Deliberate for a team tool; wrong for a personal remote control, and
  precisely what the §3 allowlist prevents.
- **Thread context is capped at 50 messages** from the mention, with the advice
  to restate anything critical. A window, not a memory.
- **Sandboxes are per-thread and ephemeral**, released when idle and rebuilt on
  reply — with the consequence that long work must *"push branches and post
  drafts mid-run"* to survive. mecha's `~/.mecha/work/<producer>/` is stable
  across runs by design, which is the better property and already built.

### The strongest signal in the survey

Three vendors independently arrived at mecha's front-door split — treat an
inbound payload as **data to extract from, never as instructions**:

- **GitHub Copilot cloud agent**: *"Only users with write access to the
  repository can trigger Copilot… **Comments from users without write access are
  never presented to the agent.**"*
- **Amp**'s event-driven automation *"extract[s] metadata, and initiate[s] a
  fresh thread — treating the event payload itself as untrusted input rather
  than instructions."*
- **Claude Code** wraps scheduled-run payloads in a delimiter that marks them as
  data.

Note the *shape* of Copilot's version, because it is the one that matters for
§3: unauthorised content is **excluded at the context boundary**, not checked at
the approver. Filtering at the approver leaves the text in the prompt where it
can still steer; excluding it means there is nothing to steer with. That is
structurally identical to `Record::for_privileged_run` having no argument that
returns the prose, and it is independent confirmation that the two-tier model in
§3 — owner, or front door, with nothing between — is the right shape rather than
a convenient one.

The corollary Claude Tag supplies: the excluded person needs an affordance, or
exclusion reads as silence. Telling the model *that* a message arrived while
withholding its body is the middle path, and it is what the front door already
does.

### The sentence to keep

GitHub, about its own approval flow for agent automation:

> **"Approvals are a workflow convenience, not a security control. They don't
> enforce a server-side boundary."**

mecha already believes this — it is why the interlock sits *ahead* of the
approver, *"because a human clicking yes is what an injection is trying to
engineer"* — but it is worth having the sentence from someone else's
documentation, because a Slack button is the most approval-shaped thing this
project will ever build, and the temptation to treat it as a boundary will be
strongest there.

Copilot's structural controls are the same idea carried further, and they are
worth knowing as a target: the agent cannot `git push` directly, cannot mark its
own PRs ready for review, cannot approve or merge, and **GitHub refuses to count
the requesting user's own approval** of a PR the agent opened for them.

### Four things to steal

1. **Surface policy denials where the human already is.** Copilot writes
   firewall blocks *into the pull request body*, naming the blocked address and
   the command that tried to reach it. For mecha: an interlock refusal, a hook
   denial, or a leak-guard block should appear as a `task_update` in the Slack
   thread — not only in a log nobody opens. A denial the human never sees reads
   as a mysterious failure, and this is nearly free given §5's mapping.
2. **Make artifacts point back at the run.** Every Copilot commit carries an
   `Agent-Logs-Url` trailer. mecha's outbox already records the drafting
   session; the generalisation is stamping that reference into the *artifact* —
   a published bundle, a file posted to Slack — so "why does this exist" is
   answerable three months later.
3. **Budget the async surface explicitly.** Codex bills cloud work against a
   *separate* quota from interactive work, and lets an in-flight turn finish
   when the limit is hit rather than losing the run. Amp is the cautionary
   case: self-scheduling agents that spawn per-minute-billed VMs with **no
   documented budget guardrail, wake-rate limit, or per-thread cost
   attribution**. A Slack thread that can start a run is a cost surface; it
   needs `max_turns` / `max_cost_usd` bound per thread, and the usage footer in
   §5 is where it becomes visible.
4. **Every state needs a documented meaning and a documented way out.**
   Copilot's REST API exposes `waiting_for_user` and `idle` and defines
   neither — so a caller cannot distinguish *waiting* from *wedged*. The TUI
   already solved this locally with watch caps that retire a stuck watch (300 s
   for a send, 1800 s for a triage); a Slack surface needs the same discipline,
   and its state machine should be written down before it is implemented.

### Two controls worth wanting, outside this feature's scope

Both are Codex's, both are the *"invert before declaring impossible"* move this
project has recorded twice, and neither is Slack work:

- **Secrets are removed before the agent phase begins** — available during
  setup, gone by the time the loop runs. Exfiltration becomes impossible rather
  than policed. (Claude Tag reaches the same end differently, injecting
  credentials at a proxy boundary the model never sees.)
- **Outbound HTTP restricted by *method*, not only by host** — an allowlist that
  permits `GET, HEAD, OPTIONS` and blocks `POST/PUT/PATCH/DELETE` stops
  exfiltration to a host you are legitimately allowed to read. mecha's sandbox
  is all-or-nothing on network today.

Recorded here because this is where the evidence was gathered; they belong in
the sandbox's backlog, not in the Slack build.

### The anti-patterns each vendor named out loud

Every one is the silently-degrading-sandbox shape this project keeps finding,
which is reason enough to write them down:

- Copilot's firewall *"does not apply to MCP servers"* — a hole exactly where
  third-party code runs.
- Copilot's content-exclusion rules apply to one surface and silently not to the
  cloud agent — a boundary that exists in one place and not another.
- A failed setup step *"skip[s] the remaining setup steps and begin[s] working
  with the current state"* rather than aborting — degrade-and-continue where
  mecha's `Sandbox::preflight` deliberately stops the run.
- Oversized images are *removed from the request* with no error.
- Amp's `dangerouslyAllowAll: false` is one of the settings that turns its
  permission system **on** — a knob whose name means the opposite of its effect.

*(Secondhand: the Codex, Copilot and Amp details in this section come from a
documentation research pass rather than from pages fetched directly here. The
Claude Code and Claude Tag material was fetched from the vendor's own docs. None
of it is load-bearing for the recommendation — it corroborates §3 and §6 rather
than establishing them.)*

---

## 10. What it costs

Roughly, and in the order the pieces bite:

| Piece | Size | Notes |
|---|---|---|
| Socket Mode client + envelope ack + reconnect | small | one new dep (`tokio-tungstenite`) |
| Identity binding, allowlist, `mecha slack link` | small | but it is the decision, not the code |
| Thread ↔ conversation ↔ workspace ↔ session ownership | medium | the `.running` marker and pid range check are the fiddly part |
| `AgentEvent` → stream mapping | small | one-to-one; buffer by size |
| `SlackApprover` / `SlackAsker` | small | copy `tui/approve.rs`; the timeout string is the subtle part |
| Outbox review in Slack | medium | reuse the TUI's id-diff scoping and the detached-child-plus-store-poll pattern |
| Files out | small | two API calls + `slack_file` |
| Files in, phase 1 (path in prompt) | small | the four download-hardening rules are most of it |
| Files in, phase 2 (`Block::Image`) | **large** | a core type change plus a `Message::text()` audit across compaction, miners, graders |
| Per-thread budgets + a usage footer | small | `Budget` already exists on `RunContext`; the work is choosing defaults and making spend visible where it is incurred (§9) |
| A written state machine for a thread | small, and do it first | every state needs a documented meaning *and* the action that resolves it, or "waiting" and "wedged" are indistinguishable (§9) |
| Trigger `notify` → Slack | trivial | `notify` already pipes text to `sh -c`; a `mecha slack notify` reading stdin gets the morning briefing onto the phone for free |

The two things that are genuinely design work rather than plumbing are the
identity binding (§3) and the approval-timeout semantics (§6). Everything else
is a transport over seams that already exist.

---

## 11. Recommended against

- **HTTP Events API**, for the reasons in §2. It buys distribution, which is not
  wanted, and costs an inbound port, a certificate, signature verification, and
  an auto-disable cliff.
- **Hosting the Slack app on the factory box** (§8). It puts a workspace
  credential on a machine assumed lost and inverts the direction of authority.
- **Exposing mecha as an MCP server to Slackbot** (§8). Different product.
- **A third trust tier between owner and stranger** (§3). Unmeasurable, and the
  front door already covers the second case.
- **Streaming thinking blocks into a channel** (§5).
- **`Answer::Always` in a long-lived connector** (§6).
- **Building `Block::Image` first** (§7). It is the largest piece and the least
  necessary; the workspace-path route delivers the capability without it.
- **Persisting a local index of workspace history.** Slack's API terms
  (effective 2025-10-10) were written against exactly that, and while the
  October revision narrowed most clauses to third-party apps rather than
  internal tools, the ban on using Slack data to *train* a model is stated
  without that qualifier. Reading messages into a model's context at inference
  time is not training; fine-tuning on them is. Worth a deliberate decision
  rather than a default, especially given that `mecha distill` exists and would
  otherwise happily turn Slack threads into episodes.

---

## 12. What to re-check before building

The Slack AI surface changed shape twice in the twelve months before this
document. These are the claims that would cost the most if stale:

- **Verified directly on 2026-08-09**: `chat.startStream` exists with the scopes
  and tiers stated; `agent_view` is mandatory for new apps and the migration is
  one-way; `assistant_thread_started` is no longer the entry point.
- **Verified from primary docs but not independently re-fetched**: the
  `task_card` block's status enum; `assistant.threads.setStatus` at 600/min with
  a two-minute auto-clear; the `slack_file` composition object; the
  `files.getUploadURLExternal` flow and its limits; the 2025 non-Marketplace
  throttle and the internal-app exemption; `trigger_id`'s three-second life;
  `response_url`'s 5 uses / 30 minutes.
- **UNVERIFIED and worth probing before depending on**: whether Socket Mode
  replays unacked envelopes across a drop; whether `chat.update` draws against
  the per-channel posting budget; `files.upload`'s actual runtime status today;
  the `image` block's pixel and byte ceilings; the exact workspace-wide ceiling
  behind "several hundred messages per minute"; whether `files[]` is a
  documented property of the base `message` event (the canonical example still
  sits on a retired subtype page).

The general rule from `docs/README.md` applies with force here: anything that
could not be verified should say so rather than carrying an old claim forward
with confidence.
