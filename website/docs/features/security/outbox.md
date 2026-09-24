---
title: The outbox
sidebar_position: 4
description: Routing outbound tool calls to a review queue — draft-only, never send, made structural.
---

# The outbox

`[outbox] tools = [...]` names tools whose calls are **staged, not executed**.
The loop intercepts the call, writes it to `~/.mecha/outbox/` as a draft, and
tells the model the draft is awaiting the user's release. Nothing leaves the
machine until a human has read exactly what would go out, in exactly the
arguments that will be used.

```toml
[outbox]
tools = ["mail__mail_send", "mail__mail_reply", "mail__calendar_create_event"]
dir = "~/.mecha/outbox"     # optional; $MECHA_OUTBOX_DIR also works
publish_tools = []          # of the above, which are publications — see below
```

The gate lives in core, which is the whole point: an email or calendar tool —
including a third-party MCP server's — needs **no knowledge of the outbox to
be covered by it.** "Draft-only, never send" stops being a promise the tool
makes and becomes a property of the harness.

What the model sees when a routed call is staged:

```
Drafted, not sent: this call is staged in the outbox as `20260805-...`.
The user will review it with `mecha outbox` and release or reject it.
Report it to the user as a draft awaiting their release — never as done —
and do not retry the call.
```

## Reviewing

```bash
mecha outbox                  # list (pending first), grouped by kind
mecha outbox show <id>        # read the draft and its source
mecha outbox show <id> --json # inspect exact arguments
mecha outbox edit <id>        # edit the prose in $EDITOR
mecha outbox edit <id> --json # edit recipients and other arguments
mecha outbox review --all     # walk the pending items, deciding each
mecha outbox approve <id>     # execute the reviewed call (`send` remains an alias)
mecha outbox reject <id> --reason "wrong recipient"
```

An id may be given as any unambiguous prefix; an ambiguous one is an error
rather than a guess. `list` marks items that were drafted in a tainted
conversation and items that have been edited:

```
20260805-a1b2  pending  mail__mail_send {"to":"dean@…","subject":"re: budget"}  ⚠ tainted  (edited)
```

`review` is the overnight-triage case: nine drafts from one run, decided in one
sitting. It walks them one at a time rather than presenting a list to
bulk-approve — batching the queue must not batch away the reading, which is the
only thing the outbox is for. `--kind` and `--via` narrow what it walks.

`approve` builds the real tool surface — MCP servers included — and calls the
tool. Delivery attempts are recorded before execution. A known failure can be
reviewed again; an uncertain outcome remains pending and blocks retries until
it has been reconciled.
Resolution rewrites the item in place rather than archiving it, so the file is
its own audit record; a rejection stays on disk as the record of the refusal.
`mecha outbox reject --all` processes every selected pending draft. An uncertain
delivery remains pending for reconciliation; other eligible drafts are rejected.
The command reports rejection/failure counts and exits nonzero if any item fails.

## Delivery recovery

If a response is lost or the sending process stops, check the destination before
retrying. An unknown outcome may already have delivered the message or event.
Record the outcome you established, with evidence:

```bash
mecha outbox reconcile DRAFT_ID --outcome delivered --evidence "Sent message m-123"
# Or, only after establishing non-delivery:
mecha outbox reconcile DRAFT_ID --outcome not-delivered --evidence "Destination history confirms no delivery"
```

Reconciliation never sends. Confirmed delivery resolves the draft; confirmed
non-delivery allows a fresh review and retry. The web outbox exposes the same
recovery. [Workflow checks](/docs/features/automation/workflows#check-the-result) count only
confirmed delivery as evidence of completion.

## A reply is shown with the message it replies to

A staged `mail_reply` carries a body and a `thread_id`, and a `thread_id` means
nothing to a reviewer — deciding *is this the right reply?* without the
original is approving unread. So `show`, the review modal and `edit` print the
message being answered underneath the draft, labelled for what it is:

```
replying to — third-party content via mail__mail_get_thread (thread_id),
not part of your draft:
  --- [dartmouth] From: … · 2026-08-17T12:00:24Z
  Subject: COSAN Lab Research Opportunity Inquiry
  …
```

It is the text the drafting run actually read, taken from that run's
transcript — never a live re-fetch — so you judge the reply against the bytes it
was written from, not today's version of the thread, and `show` needs no network.
It is labelled as somebody else's words, above the draft's own taint warning,
because that is what it is. The same applies to any tool that reads a thread or
document before replying to it, not only mail.

No block means no source was recovered — for a `mail_send` that answers
nothing there is none to find, and for a reply whose session has been pruned it
is simply gone. Those two read the same on screen today, which is worth knowing
before you take a missing block as "this reply answers nothing."

## Staging skips the interlock and the approver

Deliberately. At stage time nothing leaves the machine: the call becomes a
local file, and release requires the user to read exactly what would be sent.
There is nothing to approve, because nothing executes — the review *is* the
approval, later and out of band.

What replaces those gates is provenance. Each item records the conversation's
taint snapshot at the moment of staging, and both `show` and `send` say so
loudly:

```
⚠ this draft was written in a conversation that held private data AND
  third-party content. If anything in these arguments was not yours, an
  attacker may have put it there:
```

`send` then confirms, and **EOF counts as no** — the same rule as the terminal
approver. Silence must not send. `--yes` skips the confirmation for scripts
that have already decided.

:::note
Routing one tool loosens nothing for the rest. An **unrouted** send with the
trifecta armed still hits the interlock exactly as before; there is a test on
each side of that.
:::

## A failed staging fails closed

A call that could not be staged returns an error to the model and **never
falls through to execution**:

```
`mail__mail_send` is routed through the outbox, and staging failed: <error>.
Nothing was sent. Tell the user.
```

A full disk must not be the way around the review. For the same reason the
store is opened at startup rather than lazily at first stage, so an unwritable
outbox is a startup error instead of a mid-run surprise on the one call that
mattered.

## `args_before` is never modified

An item keeps two copies of the arguments:

| Field | What it is |
|---|---|
| `args_before` | The arguments as the agent drafted them. Never modified. |
| `args` | What a release will execute. Starts equal to `args_before`; `edit` rewrites this one. |

The pair is a **measurement**, not bookkeeping. `mecha reflect` mines
`diff(args_before, args)` on sent-with-edits items into `writing`-domain
reflections — trigger `edit`, its own reflector prompt, its own
`mined_outbox.jsonl` ledger — and `mecha learn` consolidates that domain with
its own frame: voice rules, a positive/negative mix, and never a
one-recipient rule. Your edit before sending is the clearest signal you will
ever give about how you want things written, and overwriting the baseline
would destroy it.

`edit` therefore rewrites `args` only. A parse failure keeps the original
rather than staging arguments you did not mean.

## Staging is sink-agnostic; reviewing is not

The outbox also stages a second kind of outbound action — publishing a
rendered bundle to the public surface — with the same staging, but review has
to differ, because every message affordance assumes the staged thing is prose
someone wrote.

So an item carries a kind, set at staging from `[outbox] publish_tools`:

| | `message` | `publish` |
|---|---|---|
| The reviewable object | the arguments | the **rendered page** |
| `show` | prints the arguments | names the bundle directory and `index.html` |
| `edit` | opens `$EDITOR` | **refused** — edit the source and re-render |
| Mined for `writing` rules | yes | **no** |

The last row is the load-bearing one: a `writing` reflection becomes a rule in
every future run's cached prefix, so mining the diff of a changed *path* would
teach voice rules from bookkeeping.

The kind is **config's to declare, never the tool's.** Anything unnamed is a
`message`, which is the conservative default. See
[Publishing](/docs/features/public-surface/publishing).

## An item records the jail it was drafted under

A staged call is a *deferred* tool call: the drafting run said
`{"bundle": "site"}` inside its own work directory, and release happens in
another process, hours later, from wherever you are standing. So each item
records the workspace its tool would have executed in, and `approve` — and
`show` — resolve paths there. A relative path therefore means the same file at
review and at release as it did at drafting; without that, a same-named
directory beside you would publish the wrong bytes with no error anywhere.

One case to know: an artifact written with the **built-in** file tools inside a
[Slack](/docs/features/interfaces/slack) thread lives in that thread's
workspace, while an MCP server that serves many runs has its own fixed root. Hand
such a server the artifact's **absolute** path, or it will look in a different
place.

## Subagents, and eval

**Subagents inherit the parent's route**, like hooks — or delegating would be
the way to send unstaged.

`mecha eval` forces `--no-outbox`, like MCP, hooks and provider fallbacks, for
the same reproducibility reason: a scorecard has to grade the same run
everywhere.

## A routed name that matches nothing warns on every start

```
mecha: [outbox] routes `mail__mail_sned`, which is not a registered tool —
check the spelling, or this routing protects nothing
```

A typo means the real tool executes **unrouted**, silently — the
silently-degrading-sandbox shape again. It cannot be a hard error, because a
routed tool's MCP server may legitimately be off today, so it is said out loud
on every start instead. The one exception is a name that `--tool` deliberately
excluded: that is the caller naming exactly what they want, and a warning that
fires every morning on a deliberately narrowed run is how a real typo later
gets ignored.

## Storage

One pretty-printed JSON file per item, so `$EDITOR` and `git diff` work on it.
Every rewrite is atomic, so nothing ever reads a half-written item; staging
never waits on a review you have open; and two concurrent `approve`s of the same
item cannot both send it.

## Where to go next

- [Security model](/docs/features/security) — the interlock that staging bypasses, and why that is safe.
- [Publishing](/docs/features/public-surface/publishing) — the second kind of staged action, and what review had to learn.
- [Triggers](/docs/features/automation/triggers) — where the outbox does the most work: overnight triage that leaves a review queue.
- [Learning](/docs/features/learning) — what the staged/sent diff feeds.
- [Mail and calendar](/docs/features/tools/mail) — the tools most often routed.
