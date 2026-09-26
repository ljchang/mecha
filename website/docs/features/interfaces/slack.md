---
title: Slack
sidebar_position: 3
description: Driving mecha from a phone — Socket Mode from home, an owner bound by a locally printed code, a thread as a conversation, and approvals and outbox review as cards.
---

# Slack

`mecha slack` is a **remote control**, not a chatbot. The agent stays on your
machine, with your files, your mail, your keys; Slack is a way to reach it from a
phone. A thread is a conversation, a run streams into it, tool calls that need
permission arrive as buttons, and drafts a run stages come back as review cards.

## Home dials out

The connector opens a **Socket Mode** connection *outward*. There is no inbound
port, no certificate, no tunnel, and no request signature to verify — the same
argument the factory's drain loop makes, and the reason this is a Socket Mode app
rather than a webhook.

That shape is why it works from a laptop behind NAT, and why nothing on the
internet can reach your agent by knowing an address.

## Setting it up

### 1. Create the app

An internal, single-workspace app. `scripts/slack-app-manifest.yaml` is the
whole app as a manifest — paste it into "Create New App → From a manifest". It
holds these bot scopes, the minimum for what is built:

`chat:write`, `im:history`, `im:write`, `files:read`, `files:write`

`assistant:write` is in the manifest commented out: the assistant-thread status
API it unlocks needs a paid Slack plan, and nothing requires it.

Plus an **app-level token** with `connections:write`, which is what Socket Mode
opens the connection with, and the App Home "Messages Tab" turned on with
sending allowed — without it a DM to the app is silently impossible.

You end up with two tokens: a bot token (`xoxb-…`) and an app-level token
(`xapp-…`).

### 2. Store them

```bash
export MECHA_SLACK_BOT_TOKEN=xoxb-…
export MECHA_SLACK_APP_TOKEN=xapp-…
mecha slack auth
```

The tokens are checked for shape, then **proved against Slack** before anything
is written — a token pasted into the wrong variable fails here, with a message
saying so, rather than at the first real run hours later. They are stored in
`~/.mecha/slack/`.

### 3. Say who may drive

```bash
mecha slack link
```

This prints a **one-time code on this machine**. Send it to the app in a Slack
DM, and whoever sent it is bound as the owner.

The indirection is the security model. Typing a code that was printed on the
machine proves *shell access to the machine* — where an email address proves only
what the workspace claims about it, and a workspace admin can change what it
claims. There are two trust tiers and no third: bound owners, and everyone else,
who is ignored by construction.

`mecha slack status` shows what is bound and whether the credential still works.
`mecha slack unlink` forgets the binding and keeps the tokens, so you can bind
again.

### 4. Run it

```bash
mecha slack connect                 # the connector, in the foreground
```

`connect` holds the Slack socket open and drives runs from threads. Run it by
hand first: it is the same process the unit runs, with its logs on your
terminal, which is what you want while a token, a scope or a tool surface is
still being sorted out.

Then hand it to systemd:

```bash
cp scripts/mecha-slack.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now mecha-slack
loginctl enable-linger "$USER"      # so it runs while you are logged out
```

The unit refuses to start without both the credential and the binding, loudly.

## What it feels like to use

Message the app in a DM — the app subscribes to direct messages only, so a
mention in a channel never reaches it. The answer streams in, and each tool
call becomes a small card that changes state as it runs — so a long run shows
what it is doing rather than a spinner.

- **A thread is a conversation.** A new thread is an honest clean slate; a thread
  that read a hostile web page on Monday still remembers on Tuesday. That hands
  the [trifecta interlock](/docs/features/security) the right granularity for
  free.
- **Approvals are cards**, including "allow for this run" — which arrived after
  the first real task raised seven of them.
- **Drafts come back as review cards** with Send and Reject, scoped to the run
  that staged them. This is [outbox](/docs/features/security/outbox) review from a phone,
  and it needed no home-side server at all.
- **Files go both ways.** An attachment lands in the thread's workspace and is
  named to the model as a path — so it arms taint through `fs_read`, the route
  that already exists — and what a run creates is uploaded back. An
  [image](/docs/features/interfaces/images) also goes on the turn itself, so "what is
  wrong with this chart" is a question you can ask from a phone.
- **What a run creates comes back as attachments, up to five.** Past that,
  the rest are named in the thread rather than sent.
- **`mecha slack notify`** reads stdin and DMs it to you, which puts a
  [trigger's](/docs/features/automation/triggers) morning briefing on your phone for the
  price of a config line.

## Command words

A few DM messages are commands rather than prompts. Each is matched before the
text can reach the model, and only after the owner check, so a stranger's
`doctor` never gets looked at:

| Message | What comes back |
|---|---|
| `doctor` | the same findings `mecha doctor` prints, with a button on the ones a phone can fix |
| `queues` | the review backlog across the stores, read-only |
| `tasks` | the task board, with **Done** and **Drop** (and **Next** for an inbox capture) on each row; a Done or Drop is recorded as a closure, and the reply carries its appraisal |
| `task <text>` | captures `<text>` onto the board |
| `triggers` | the schedule, with **Run**, **Cancel**, **Enable** or **Disable** as each row's state allows |
| `note <text>` | captures `<text>` into the knowledge graph |
| `review [now\|later\|auto]` | shows or sets when staged drafts stop for review — for the thread it is sent in, or for new threads if sent top-level |

The bare words (`doctor`, `queues`, `tasks`, `triggers`) must be the whole
message — "show my tasks please" is a prompt. A capture is deterministic on
purpose: a note *asked* of the model may or may not get written, and a capture
that depends on the model's mood is not a capture.

Every button carries a fixed verb and an object id, never a command line. The
command is rebuilt by code from the store at tap time and run as the matching
`mecha …` child process, so no model output and no message text sits between a
finding and what a tap executes.

## Configure the tool surface

```toml
[slack]
tools = [
  "fs_read", "fs_write", "fs_edit", "fs_list", "shell", "todo",
  "web_search", "http_fetch",
  "factory__bundle_render", "factory__bundle_publish", "factory__bundle_list",
]
default_mode = "ask"        # or "allow", "read-only"
max_concurrent = 3          # threads with a run in flight at once
approval_timeout_secs = 600 # then the call is refused as unanswered
max_turns = 40
# max_cost_usd = 5.00       # unset by default: no per-run ceiling
stream_flush_chars = 800    # flush a streamed chunk at this much text…
stream_flush_ms = 1000      # …or this long, whichever comes first
max_upload_mb = 25          # Slack allows 1 GB; a remote control does not need it
```

At `max_concurrent` the connector refuses and says so rather than queueing: a
run that starts twenty minutes later against a workspace that has moved is
worse than an honest refusal. An approval that times out is `Blocked`, never a
denial by the user — see [hooks](/docs/features/security/hooks) for why that distinction
is in the type.

**`[slack]` is stripped from project config layers**, with a warning, and loads
from the global config only. A `mecha.toml` arrives with a cloned repository,
and Slack is the remote control: nothing in this table grants access — who may
drive lives in the binding store — but a repo must not get to widen the default
mode or the budget of runs you drive from your phone. There is a test named on
it.

`tools` is worth setting rather than leaving empty. Empty means "everything
configured", and measured on the first live run, the schemas of every wired MCP
server cost **~7–8k input tokens per turn** before any work happened — against a
32k window whose compaction threshold is 21,845, a run starts a third of the way
there. A phone rarely needs the mail, the calendar and the factory at once.

Names are exact, never globs. Note that `http_fetch` is also what a `research`
subagent needs — without it, that subagent is not registered, and the only sign
is a line in the connector's journal.

Slack runs load no [skills](/docs/features/learning/skills), for the same reason
`ask_user` is absent (below): one agent serves every thread, and a skill loaded
in one thread would change the tool surface of the others.

## Getting a file out

A box you reach over SSH can make a chart you cannot look at. `mecha slack
send` — and `/send <path>` inside the TUI — uploads one file into your own DM,
where a phone can open it:

```bash
mecha slack send results/accuracy.png
```

```
/send results/accuracy.png
```

Two things about it are deliberate. **The destination is not an argument** —
it is your DM, read from the binding, and no flag moves it; that is what
lets the agent surface a chart itself without the ability to send one anywhere
else (see `show_file`, below). And **the TUI's `/send` resolves the path through the run's
jail**, so it reaches what the session can reach and nothing else. To send
something from outside the workspace, `!cp` it in first.

`[slack] max_upload_mb` (25 MB) caps this, and caps attachments coming the
other way with the same number.

**The agent's own version is the `show_file` tool.** In the TUI, a run can put
a workspace file into the session's [`/remote-control`](#remote-control-one-session-two-places)
thread in your DM — a chart it just rendered on a headless box, rather than a
sentence describing it. The model names a path (through the jail, like every
other path) and never a destination, which comes from the attach record. It is
declared read-only, so it asks no approval, and it is not routed through the
[outbox](/docs/features/security/outbox): the destination is the owner's own
DM, and approving a draft in order to see the picture you asked for would be
review going in a circle. When the session is not attached it refuses and tells
the model to say where the file is instead.

## Remote control: one session, two places

`/remote-control <name>` in the TUI gives a live terminal session a named
thread in your DM. What the run does appears in both places, what you type
appears in both places, and files move both ways:

```
/remote-control lab      # attach, or pick the thread up again
/remote-control          # what this session is attached to
/remote-control off      # detach; the thread and its history stay
```

**A name is durable and its thread is forever.** `/remote-control lab`
tomorrow posts into the same thread as today, which is what lets a line of
work accumulate in one place. Detaching does not delete the record — that
record is *how the thread is found again*.

**Typing in the thread reaches the session**: steering if a run is in flight,
a new turn if not. Files you drop there land in the session's workspace under
`./inbox/`, and only the *path* is given to the model — it reads them with
`fs_read`, so the taint arms through the tool that already declares
`private_data` rather than a parallel path.

**Slash commands and `!` escapes stay at the terminal.** They are not prompts:
`/model` rebuilds the agent, `/clear` drops the conversation and its taint,
and `!` runs a shell command with no approver in front of it. Those are
affordances of sitting at the machine.

Inbound needs `mecha slack connect` running; output does not. `mecha slack
remote` lists what this machine mirrors, and `--sweep` cools any attachment
whose session has gone.

**A screenshot dropped into the thread is looked at, not merely filed.** The
TUI that owns the session downloads it into the workspace under `inbox/` and
puts the image on the turn, so the model gets both: pixels, and a path it can
`shell` or `fs_read`. This is the door that exists *because* the terminal's own
one cannot work over SSH — a file dropped on the prompt pastes the path on your
laptop, which the machine at the other end resolves to nothing. See
[Images](/docs/features/interfaces/images#from-slack).

## Links never unfurl

Every message mecha posts or edits in Slack goes out with unfurling off, and
there is no parameter to turn it on. An unfurl is Slack fetching a URL the moment a message
lands, so a link the model wrote would become an outbound request that no tool
call made and the [trifecta interlock](/docs/features/security) never saw — the
same reason `http_fetch` counts as a send even though it only reads. Because the
setting belongs to the transport rather than to each message, no call site can
forget it.

## Two things it deliberately does not do

- **`ask_user` is absent.** It is a *tool*, and the tool registry belongs to the
  agent — one of which serves every thread — so a shared `ask_user` could not
  know which thread was asking. The approver rides on the run and so is
  per-thread for free; the tool cannot be, without an agent per thread.
- **MCP tools do not honour the per-thread jail.** Servers are spawned once with
  the agent, so they cannot follow a per-thread workspace. They are rooted at the
  `slack` producer directory, of which every thread's jail is a subdirectory, so
  at least the two agree about where a relative path points — a mismatch that
  once cost a run five turns and a `shell` workaround. Isolation *between*
  threads is not there.

Both want the same fix — an agent per thread, and an MCP startup per thread with
it — and neither is pretended away.

## Recovering from restarts

Slack rotates connections every few hours with about ten seconds' warning, so
reconnect is **make-before-break**: the replacement opens before the old one
drains, and no frame has nowhere to land.

If the process dies mid-run, `mecha slack sweep` marks threads whose run did not
survive, so none is left showing "working…" forever. The connector does this on
startup; the command is the same pass by hand. `mecha slack threads` shows what
state each thread is in and what would resolve it.

## Where to go next

- [The outbox](/docs/features/security/outbox) — what review means, and why it is separate
- [Triggers](/docs/features/automation/triggers) — scheduled runs, and `notify`
- [Security model](/docs/features/security) — what a thread's taint is doing
