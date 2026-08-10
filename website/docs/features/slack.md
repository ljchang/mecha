---
title: Slack
sidebar_position: 18
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

An internal, single-workspace app. It needs these bot scopes:

`chat:write`, `assistant:write`, `im:history`, `app_mentions:read`,
`files:read`, `files:write`, `users:read`, `users:read.email`, `commands`

plus an **app-level token** with `connections:write`, which is what Socket Mode
opens the connection with. `users:read.email` is used once, at link time, and for
nothing else.

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

Message the app, or mention it in a thread. The answer streams in, and each tool
call becomes a small card that changes state as it runs — so a long run shows
what it is doing rather than a spinner.

- **A thread is a conversation.** A new thread is an honest clean slate; a thread
  that read a hostile web page on Monday still remembers on Tuesday. That hands
  the [trifecta interlock](/docs/features/security) the right granularity for
  free.
- **Approvals are cards**, including "allow for this run" — which arrived after
  the first real task raised seven of them.
- **Drafts come back as review cards** with Send and Reject, scoped to the run
  that staged them. This is [outbox](/docs/features/outbox) review from a phone,
  and it needed no home-side server at all.
- **Files go both ways.** An attachment lands in the thread's workspace and is
  named to the model as a path — so it arms taint through `fs_read`, the route
  that already exists — and what a run creates is uploaded back.
- **`mecha slack notify`** reads stdin and DMs it to you, which puts a
  [trigger's](/docs/features/triggers) morning briefing on your phone for the
  price of a config line.

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
denial by the user — see [hooks](/docs/features/hooks) for why that distinction
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
subagent needs — without it, that subagent silently stops being registered.

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

- [The outbox](/docs/features/outbox) — what review means, and why it is separate
- [Triggers](/docs/features/triggers) — scheduled runs, and `notify`
- [Security model](/docs/features/security) — what a thread's taint is doing
