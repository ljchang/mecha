---
title: The outbox
sidebar_position: 7
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
mecha outbox show <id>        # the exact arguments a release would execute
mecha outbox edit <id>        # open those arguments in $EDITOR
mecha outbox review --all     # walk the pending items, deciding each
mecha outbox send <id>        # execute the call, for real, and mark it sent
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

`send` builds the real tool surface — MCP servers included — and calls the
tool. A failed release records the error and leaves the item **pending**: the
draft is still good, the delivery was not, and the next `send` retries.
Resolution rewrites the item in place rather than archiving it, so the file is
its own audit record; a rejection stays on disk as the record of the refusal.

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
rather than staging arguments you did not mean, and the lock is taken only
*after* `$EDITOR` exits.

## Staging is sink-agnostic; reviewing is not

The outbox generalised to a second kind of outbound action — publishing a
rendered bundle to the public surface — **with no change to `outbox.rs` at
all**, which was the design goal. Every one of its *review* affordances broke,
because all three assume the staged thing is prose someone wrote.

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
[Publishing](/docs/features/publishing).

## An item records the jail it was drafted under

A staged call is a *deferred* tool call, and a tool call means nothing apart
from its workspace. The drafting run said `{"bundle": "site"}` inside
`~/.mecha/work/<producer>/`; `send` runs in another process, hours later, from
wherever the reviewer is standing.

So the item records a workspace and the release rebuilds the tool surface rooted
there. An absolute path fails loudly in the wrong place; **a relative one is
worse**, because a same-named directory beside the reviewer publishes the wrong
bytes with no error anywhere.

**Which workspace is recorded is the subtle part: the tool's own fixed root when
it has one, and the run's otherwise.** A tool with a fixed root — an MCP server
spawned once for many runs — resolved its paths against that root *at draft time
too*, so a release that re-rooted it anywhere else would execute a different call
than the one the model made. Note that this is not always the narrower of the
two. It was a live bug in exactly the direction that surprises: [Slack](/docs/features/slack)
threads are jailed to subdirectories of the producer root the MCP servers run
in, staging recorded the *thread* jail, and every Slack publish therefore failed
containment on release, forever.

The mirror case is the residual hazard worth knowing: an artifact authored with
the **built-in** fs tools lives in the thread's jail, so handing its relative
name to a fixed-root server names a different place. Give that server the
absolute path.

**`show` resolves through the recorded jail too, not only `send`.** The display
forgot the jail long after the executor learned it, which reported a draft's
source file as gone — and, in the symmetric case, would have printed and offered
to open a same-named file beside the reviewer as though it were the draft's.
A reviewer reading one file while approving another is the failure this whole
surface exists to prevent, so every surface that touches a staged path resolves
it the same way.

A batch release builds one surface per distinct workspace, lazily, so the
ordinary nine-replies-from-one-run case still starts the MCP servers exactly
once. An item staged before the field existed releases against the reviewer's
workspace, which is what it always did.

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

The store follows the learning store's rules: one pretty-printed JSON file per
item, so `$EDITOR` and `git diff` work on it; temp-sibling-and-rename for
every rewrite, so a reader never sees a half-written file; and an advisory
`flock` for read-modify-write paths, taken before reading the state acted on
and **never held across `$EDITOR`**.

**Staging takes no lock at all.** A fresh item is a fresh file with a unique
id, so there is no state to race on — and the agent loop must never block on a
human's review session. `send` holds the lock across execution instead, so two
concurrent sends of the same item cannot both pass the pending check and
double-fire.

## Where to go next

- [Security model](/docs/features/security) — the interlock that staging bypasses, and why that is safe.
- [Publishing](/docs/features/publishing) — the second kind of staged action, and what review had to learn.
- [Triggers](/docs/features/triggers) — where the outbox does the most work: overnight triage that leaves a review queue.
- [Learning](/docs/features/learning) — what the staged/sent diff feeds.
- [Mail and calendar](/docs/features/mail) — the tools most often routed.
