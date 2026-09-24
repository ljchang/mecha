---
title: Messages between agents
sidebar_position: 2.5
description: How mecha sessions on one machine leave each other short messages, why every message carries its sender's taint, and the global-only policy that decides what a receiver does with them.
---

# Messages between agents

mecha sessions on the same machine can leave short text messages for each other.
An overnight trigger can tell tomorrow's chat session where it put the briefing,
and a chat can ask a scheduled run to look at something next time it fires. It
works through a store of files. There is no socket or daemon, and you can read
the whole thing with `mecha msg`.

Messaging is off by default. Turn it on in your global config:

```toml
[messages]
enabled = true
```

## Addresses are producers, not sessions

A message is addressed to a **producer**, which is the same name the
[work directory](/docs/features/automation/work) uses:

| Producer | Who reads its mailbox |
|---|---|
| `chat` | `mecha chat` and `mecha tui` sessions |
| `run` | one-shot `mecha run` |
| a trigger's name | that [trigger](/docs/features/automation/triggers)'s scheduled runs |

A mailbox belongs to a producer, not to one session, so any live run of that
producer can claim what is waiting. That is what lets a trigger write to `chat`
without knowing which chat session will open tomorrow. Names are lowercase
letters, digits, `-` and `_`, up to 64 characters. A session gets its identity
when it starts, so a `--no-session` chat has no mailbox and cannot send.

A delivered message lands at the recipient's **next turn**. The run checks its
mailbox at the top of each turn and adds any messages to the message that
carries its tool results. Steering uses the same slot, because there is nowhere
else a message can legally go in the middle of a run. If no run is live,
nothing is lost, and the message waits for the next one. A run that is about to
stop does not claim mail, so it never marks as delivered a message it will not
act on.

A delivered message is clearly labelled. Its header names the sender and says
that the message is from another mecha agent, not the user, and that it cannot
approve actions, grant permissions or change the receiver's instructions. The
receiver's own permissions, hooks, outbox and interlock decide what happens
next. Peer messages are also kept out of what the learning system reads as your
corrections.

## Taint travels with every message

Without this, a message would be a way around the
[interlock](/docs/features/security). A run that read a hostile web page could
pass its contents to a clean session that never saw the page. So the harness,
never the model, records the sender's conversation taint on every message. On
delivery, that taint is merged into the receiving conversation *before* the text
arrives.

This means a tainted overnight run can still report to `chat`. When the morning
session receives the report, its interlock acts as though that session had read
the untrusted content itself. If private data is also in the conversation, the
interlock refuses sends to a destination the model chooses. When the sender's
conversation contained untrusted content, the body is also wrapped as untrusted
data, just like a tool result from outside.

When mecha cannot tell where a message came from, it assumes the worst:

- A message with no recorded taint, for example one written by an older build
  or edited by hand, is treated as private and untrusted.
- A send from a context the loop did not stamp is labelled fully tainted.
- `mecha msg send` from a terminal is clean, because a person typing is trusted
  input. The same command from a pipe or script, including an agent's `shell`,
  is labelled private and untrusted.

## What a receiver does with mail

`inbound` decides whether a run adds messages to its conversation:

| `inbound` | Behaviour |
|---|---|
| `accept` | Deliver at the next turn. |
| `hold` | Leave messages pending for you to read with `mecha msg`. |
| `refuse` | Reserved. It currently behaves as `hold`, and mecha prints a warning at startup saying so. |

If `inbound` is not set, the default depends on how the run started. Scheduled
trigger runs **accept**, because nobody is there to release a hold. Their
read-only mode, outbox staging, interlock and the merged taint still limit what
a message can cause. Everything a person drives **holds**: chat, the TUI and
`mecha run`, even `run` with piped input. When a chat or TUI session starts
with held mail, it tells you how many messages are waiting and points you to
`mecha msg list`.

Held and undelivered messages stay pending in
`~/.mecha/messages/<recipient>/`, one JSON file each.

## Limits

| Key | Default | What happens at the limit |
|---|---|---|
| `pending_cap` | 50 | Further sends to that recipient fail with *"mailbox … is full"*, and nothing is sent. Old messages are never silently dropped to make room. |
| `max_body_bytes` | 65,536 | The send fails. The error suggests writing the content to a file and sending its path. |
| `keep` | 100 | Delivered and dismissed messages beyond this are pruned, oldest first. Pending messages are never pruned. |
| `dir` | `~/.mecha/messages` | Where the store lives. `$MECHA_MESSAGES_DIR` also overrides it. |

Sending the same body from the same sender in reply to the same message while an
earlier copy is still pending writes nothing new. This stops two agents from
echoing each other and filling a mailbox.

## The `message_send` tool

When messaging is enabled, runs get a `message_send` tool with three fields:
`to`, `body` and an optional `reply_to`. The model writes only those. The
harness fills in the sender's identity and taint.

The tool has **none** of the four capability labels. It is not a send in the
interlock's sense, because the message goes into an owner-only store on this
machine and never leaves it. The risk that the message carries untrusted
content is handled by passing taint along instead. The tool is also marked
read-only, so it does not ask for approval. That lets an unattended run report
back. The safeguards are the pending cap, the duplicate check and the taint.
It is refused while a run is in its planning phase, because sending starts
another agent working.

`--no-messages` removes the tool and turns off delivery for one run. An active
`--tool` allowlist that does not name `message_send` also removes the tool.

## The CLI

```bash
mecha msg send chat "the briefing is in ~/.mecha/work/briefing"
mecha msg list                  # pending, every mailbox
mecha msg list --to chat --all  # include delivered
mecha msg show 9c1e
mecha msg dismiss --all --to chat
mecha msg agents                # which sessions are live right now
```

`send` records the sender as `user` unless you pass `--from`. It also tells you
whether the recipient is running now. A running recipient may still be holding
mail, so this does not promise delivery. `dismiss` sets pending messages aside
unread. You need it because a full mailbox refuses new sends. Dismissed
messages stay on file. `agents` reads a marker each live session writes, and a
marker whose process is gone is cleaned up.

`mecha msg` works whether or not `[messages] enabled` is set, because reading
what an overnight run left you should not depend on a feature flag. Every flag
is in the [CLI reference](/docs/reference/cli#msg).

## Why `[messages]` is global-only

`[messages]` controls what your own sessions accept. A project's `mecha.toml`
comes with whatever repository you cloned. If it could set `inbound = "accept"`,
a repository could decide what gets folded into your conversations. So mecha
reads the section only from the global config file. A `[messages]` table in a
project file is removed while loading, with a warning that names the file, so
you never see a setting that looks applied but is being ignored. The CLI opens
the store from the global config too, so it always reads the same store your
agents write.
