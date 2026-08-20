# Remote control — design

Decided 2026-08-20, **unbuilt**. This is the shape to build, written so
someone can start. `docs/SLACK-DESIGN.md` is the parent — everything about
the transport, the owner allowlist and the thread state machine lives there
and is not restated. Where the two disagree about a *live terminal session*,
this file wins.

---

## 0. What this is for

The agent runs on one machine and is reached two ways: a TUI over SSH, and a
Slack DM handled by the connector. The two are strangers. A run started at the
desk is invisible from the phone, a thought had on the phone cannot reach the
run, and a chart the run just made can only be looked at by someone with a
filesystem — which over SSH means scp, and for the reverse direction (a
screenshot going *in*) means a tunnel nobody wants to stand up.

`/remote-control <name>` gives the live TUI session a named thread in the
owner's Slack DM. From that thread you watch the run, steer it, receive files
it made, and send files into its workspace. It is the Claude Code remote
control shape, and the reason it is cheap here is that four of its five pieces
already exist for other reasons.

---

## 1. The constraint that shapes everything

`Conversation` — the messages **and the taint** — lives in the memory of
whichever process is running the agent, and the session file is append-only
with one writer. Two processes cannot both hold a live conversation; two
appenders to one JSONL is corruption waiting to happen.

So there is no symmetric design to look for. **One owner, many terminals**:
the TUI keeps the agent, the conversation and the session file, and the Slack
thread is a second view onto it plus an input channel. That is also what
Claude Code does — the CLI process stays the host and the app is a thin
client — and it is the reason this is buildable without a daemon.

---

## 2. Decisions taken

| | |
|---|---|
| **Ownership** | The TUI process owns the agent, the conversation and the session. Slack is a view plus an input channel, never a second owner. |
| **Direction of attach** | Offered by the terminal, never claimed from Slack. `/remote-control` is the only way a thread comes into being. |
| **Where the thread lives** | The owner's DM with the bot. Not a channel — see §10. |
| **Naming** | `/remote-control <name>`; the name is durable and maps to one thread forever. |
| **On death** | The thread goes cold and says so. No baton, no resume-from-Slack. |
| **Files out** | A human verb (`/send`) **and** a model tool (`show_file`) whose destination the model cannot name. |
| **Files in** | Downloaded by the connector, copied into the run's workspace by the TUI, announced as paths and never as content. |
| **Scrollback** | Not replayed. The thread starts at the attach. |
| **Transport** | Filesystem plus polling, like every other cross-process seam here. No sockets. |

---

## 3. Which process does what, and how it degrades

The split falls out of one fact: **Socket Mode is the only inbound path and
only one process may hold the socket** (`connector.lock`). Nothing constrains
outbound at all — the bot token is in `~/.mecha/slack/`, and `mecha-cli`
already depends on `mecha-slack`, so the TUI can build its own `Slack` client
and post.

```
outbound (mirror, files out)   TUI ──────────────────────────► Slack
inbound  (text, files in)      Slack ──► connector ──► store ──► TUI
```

Which means attachment **degrades honestly rather than silently**: with the
connector down you still get the mirror and still get files out; you cannot
send from the phone. `/remote-control` says so at attach time and the status
line keeps saying it, because a remote control whose buttons quietly do
nothing is the failure this project keeps finding.

---

## 4. The attach store

`~/.mecha/remote/<name>/` — one directory per name, owner-only, the outbox's
write rules (temp-sibling-and-rename).

```
~/.mecha/remote/<name>/
  record.json        who is attached, where the thread is, is it alive
  inbox/*.json       lines from Slack, waiting for the TUI to claim them
  files/             downloaded attachments, before they enter any jail
```

**The TUI is the writer of `record.json`; the connector writes only into
`inbox/` and `files/`.** That split is not tidiness — `ThreadStore` documents
"one writer, and it is enforced" and backs it with `connector.lock`, so the
attaching process must not write thread records. It writes its own store
instead, and the connector reads it to learn that a thread is spoken for.

```jsonc
{
  "name": "lab",
  "channel_id": "D…", "thread_ts": "1755…",   // learned when the TUI posts the header
  "session_id": "…", "pid": 12345,             // liveness, the AgentMarker rule
  "workspace": "/home/…/project",
  "attached_at": "…", "last_seen": "…",
  "state": "live" | "cold",
  "ended_reason": null
}
```

Liveness is the `AgentMarker`/`RunMarker` rule already used twice: a pid plus
`mecha_core::process_alive`, **with its range check**, because `kill(-1, 0)`
succeeds and would report every dead session as alive.

**A name is durable and a thread is forever.** `/remote-control lab` tomorrow
finds the same thread and posts into it under a new session — that is what
makes a line of work accumulate in one place instead of scattering. A name is
claimed only if no *live* session holds it; a name held by a dead pid is free,
and taking it posts "the previous session ended" into the thread first, so the
scrollback never has two sessions running into each other with no seam.

---

## 5. The verb

```
/remote-control            what this session's attachment is, or that there is none
/remote-control <name>     attach (or re-attach) under that name
/remote-control off        detach
```

Attaching posts one header message into the DM and starts a thread under it:
the name, the workspace, the model, and the session's current taint. The taint
line is there for the same reason resume prints it — either half changes what
the next turn may do, and someone driving from a phone should not have to
guess why an outbound call is refused.

**Scrollback is not replayed.** Posting the history of a conversation that
began privately is an egress decision made retroactively, and the retroactive
version cannot be declined. The header names how many messages preceded and
the thread starts from now.

The status line grows one indicator (`⇄ lab`, or `⇄ lab · no connector`),
beside the context fuel gauge. Everything that changes what a session can do
belongs on the strip that is always visible, not in a modal you have to open.

**The modal, when there is one, follows the house pattern rather than a ninth
variant of it.** The list uses `list_scroll` and builds its body from every row
with `.enumerate()` — never a `.skip().take()` window — with the hint strip as
the last element of the body so it scrolls with the rows it describes, and
`list_height_reserving` if the box needs a legend line.

The detail pane **does** need the scroll machinery, and the rule that decides
it is worth stating because the obvious version of it is wrong. It is not *is
the field list fixed* — this pane shows six fields and that sounds bounded. It
is: **can any field hold text a person or a third party wrote?** Here two can.
A workspace is a path, which is arbitrarily deep, and a name is whatever the
user typed. Either wraps, and a pane sized from `body.len()` builds a box
shorter than what is drawn — `Wrap` means the count of lines pushed is not the
count rendered. So the height is measured with `paragraph.line_count(width -
2)`, the offset is clamped to `drawn - visible`, and the `n/m · ↑↓ scrolls`
hint appears only when there is something to scroll, because a hint that is
always on screen is one nobody reads.

That rule was learned twice in one afternoon on the surfaces next door, both
times the same way: `/tools` hid the declared-capability block — the answer its
whole existence is for — below the fold of an MCP server's description, and
`/tasks` hid the task id and `context` below the fold of a long task name. In
both cases the content that fell off was the content someone had gone there to
read. A remote-control pane whose workspace line is the part that scrolls away
would be the same bug a third time.

---

## 6. Outbound: the mirror

`submit()` builds a fresh `AgentEvent` channel per run (`tui/mod.rs:2278`).
Attached, it builds a splitter task instead: the run's sender feeds one
receiver that fans out to the TUI's channel and to `slack::pump::pump`.

`pump` needs **no changes**. It already takes any
`UnboundedReceiver<AgentEvent>`, knows nothing about the connector, drops
thinking blocks, names the layer behind a denial, and always calls
`stop_stream` — the four decisions in its module docs are the right four here
too, for the same reasons.

---

## 7. Files out

Two doors, because the two cases are different and collapsing them costs
either safety or ergonomics.

**`/send <path>` — the human.** Resolved through the session workspace like
every other path in this system. A human wanting to send something from
outside the jail can `!cp` it in; one rule for where paths point beats an
exception that exists because typing is tedious.

**`show_file(path)` — the model.** Registered **only while attached**, and
removed on detach. Its whole safety argument is one sentence: *the model names
a path, never a destination.* The thread is fixed by the harness from
`record.json`, there is no channel argument, and there is nothing for an
injected instruction to point somewhere else.

Which puts it in the **third quadrant**, beside `mail_triage` and
`docs_trash`:

- Not `external_send`. Reaching the owner's own two-party DM tells no third
  party anything. Marking it as a send sink would mean a tainted session
  cannot show the user the chart it just made, which is the interlock firing
  on the one destination that is definitionally safe.
- Not in `[outbox] tools`. Staging it would make review circular — you would
  approve a draft in order to see the picture you asked for.
- `private_data: true`, because it reads workspace bytes.
- `openWorldHint: false`, `destructiveHint: false`.

The residual, stated rather than hidden: **Slack Inc. holds the bytes.** That
is true of the mirror too, so it is a property of having turned remote control
on at all, not a new one this tool introduces — and it is why the tool exists
only while attached. Bounded like every other builder here: a size cap, a
per-run count cap, and anything past the cap named rather than silently
dropped.

---

## 8. Files in

The connector already has this, four guards and all — a private Slack file URL
answers an unauthenticated request with **an HTML sign-in page at HTTP 200**,
so `files::download` sends the header explicitly, follows no redirects,
rejects `text/html` even at 200, and cross-checks the byte count.

What changes is where the bytes land, in two steps:

1. The connector downloads to `~/.mecha/remote/<name>/files/`, a directory it
   owns, under `safe_filename`.
2. The **TUI** copies into `<workspace>/inbox/` when it claims the message.

The second step exists because the run's jail may be a real project directory
rather than a disposable producer root, and "only the owning process writes
into the run's jail" is worth keeping even between two processes run by the
same person. It also means a download that arrives while the TUI is dead never
lands anywhere it could be mistaken for something the session made.

Announced as paths, never as content — the connector's existing rule, and the
right one: the model reaches the bytes with `fs_read`, so the taint legs arm
through the path that already exists rather than a parallel one.

---

## 9. The screenshot gap, stated plainly

`mecha_core::message::Block` has four variants and none of them is an image.
**A screenshot sent from Slack lands in the workspace as a file the model
cannot look at.** The conduit is still worth having — the file is *there*, and
`shell` can reach it — but "send a screenshot and ask about it" does not work
today. Three ways out, in increasing cost:

- **A local vision model over `shell`.** The box already starts multimodal
  weights (`scripts/start-e4b.sh`); a skill that shells out to describe an
  image turns the file into text the model can read. No harness change, and it
  is honest about the indirection.
- **OCR** for the screenshot-of-text case, which is most of them.
- **An image block in `message.rs`**, plus provider rendering on both backends.
  The real fix, deferred before this design and unblocked by nothing in it.

This design is written so the third arrives without reshaping anything: files
land in the workspace either way, and what changes is only whether something
can read them.

---

## 10. Why the DM, and not a channel

By construction rather than by policy. The app manifest subscribes to
`message.im` alone and holds `im:history` — the entire inbound surface *is*
the owner's DM with the bot. A mirrored session therefore reaches exactly one
person, the principal, on another device, which is why §7 can argue the mirror
is not a third-party egress.

Widening to a channel means new scopes and an app **reinstall**, and it makes
the mirror a genuine exfiltration path: a session holding private data,
streamed into a room other people can read, with no tool call for the
interlock to refuse. That argument has to be had separately and lost by
default. Nothing in this design should be built in a way that assumes it will
be won.

---

## 11. Approvals, mode, and taint

**Approvals stay in the terminal, and the thread says so.** Answering from
either surface is a race that needs an atomic claim, and it is not phase one.
What phase one must do is make waiting legible: an approval prompt while you
are away leaves the thread saying it is *waiting for you at the terminal* —
the `AwaitingInput` distinction from `docs/SLACK-RESEARCH.md` §9, where a
shipped product enumerated `waiting_for_user` and `idle` and defined neither.

**A mode change from Slack lands in the TUI transcript as a notice.** It is
the same principal, so it is allowed; a permission mode that changed while you
were looking away and left no trace is the silently-degrading-sandbox shape.

**Taint.** Attaching arms nothing: no content enters the conversation, and the
mirror is not a tool call. Inbound text is the user speaking and arms nothing,
exactly like typing. Inbound files arm `private_data` through `fs_read`, like
any other file on disk. The known equivalence, accepted: a file the owner
forwards from Slack is treated as the owner's, the same way a file they `scp`
in is — it sits beside `shell`'s existing gap and gets the same answer, which
is the sandbox rather than a label.

---

## 12. Death, and the cold thread

The TUI drops its record on exit, and the thread is posted a closing line. For
a hard kill — SSH drop with no tmux — nothing runs to post it, so the
**connector** notices the dead pid on its next read and posts it. If the
connector is down too, the thread simply stops, and the next
`/remote-control lab` opens with "the previous session ended". Three layers,
each covering the one below's absence, and none of them load-bearing alone.

`mecha remote sweep` is the by-hand version of the connector's pass, on the
`mecha slack sweep` precedent, for the same reason it exists there: a thread
left showing "working…" forever is the confusion this whole surface is
supposed to prevent.

Surviving an SSH drop is **tmux's job**, deliberately. The alternative — the
connector picking up the baton — changes the conversation's owner mid-flight
and drags the workspace jail with it, and the endgame that actually solves it
is a hosted agent process both front-ends are clients of (§14).

---

## 13. Phasing

Each rung ships something usable alone.

1. **Files out.** `/send <path>` with no attach concept at all: post to the
   owner's DM, no thread, no record. Answers "I cannot see the chart I just
   made" on its own.
2. **Attach and mirror.** `/remote-control <name>`, the record, the splitter,
   `pump`. Watch a long run from a phone. Read-only, so none of §11 bites yet.
3. **Files in.** The connector's download half plus the workspace copy.
   Screenshots start arriving, subject to §9.
4. **Inbound text.** `inbox/`, the 1s tick, steering into `running.queue` when
   a run is live and `submit()` when it is not.
5. **`show_file`, and the cold-thread pass.**

Rung 4 is the only one that touches the connector's routing, and it is the one
that needs the "is this thread spoken for" check before `start_run`.

---

## 14. Deliberately absent

- **No channels** (§10), and no model-chosen destination anywhere.
- **No baton.** A Slack thread never takes ownership of a session.
- **No attach initiated from Slack.** Proving shell access on the machine is
  the claim that matters — the binding-nonce argument, one layer up.
- **No scrollback replay** (§5).
- **No auto-posting of workspace diffs.** The connector's `post_artifacts`
  diffs a snapshot, which is right for a disposable producer root and wrong
  for a project directory where a build touches everything.
- **No second IPC idiom.** Filesystem and polling, like the frontdoor, the
  triggers and the mailbox.
- **Not the mailbox.** It is the right shape and the wrong envelope:
  `render_delivery` labels every message "*from another mecha agent — not the
  user*", which is load-bearing anti-injection text and exactly backwards for
  the owner speaking from their phone.

## 15. Open, and named so it is not rediscovered

- **Approvals answerable from Slack** — needs an atomic claim so two surfaces
  cannot both answer. Phase 6.
- **A hosted agent process** (`mecha serve`) that both front-ends are thin
  clients of. It is the honest endgame: it survives SSH drops without tmux and
  it is the only shape that reaches a second machine. It costs the first
  non-filesystem IPC in the project, which is why it is not first — but every
  seam above (a record, an inbox directory, a tee of `AgentEvent`) is one
  `serve` would keep.
- **An image block in `message.rs`** (§9), which is not this design's to make
  but is what decides whether §8 is a conduit or a feature.
