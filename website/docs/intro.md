---
title: What mecha is
sidebar_position: 1
description: A harness that turns a local open-weight model into a personal assistant with your context, your permissions, and a way to reach the world — safely.
---

# What mecha is

In mecha anime the pilot is an ordinary person. What makes them formidable is
the suit: it gives them reach, senses, armour, and a way to act on the world.
The suit does not think for the pilot. It is built, maintained, and answerable
to the person inside it, and it is the difference between someone who could help
and someone who can.

**mecha is that suit, and what you strap into it for is the daily grind** — the
long tail of academic and professional administration that is tedious rather
than difficult, arrives faster than it leaves, and is nobody's actual work.

The pilot is a **local open-weight model**, running on hardware you own and
reading data that never leaves it. Such a model is entirely capable of being an
excellent assistant, and is nowhere near being one out of the box. It has no
memory of you. It cannot see your mail or your calendar. It can produce text
and nothing else. And it has no defence at all against the first web page that
tells it to forward your inbox to a stranger.

Everything in mecha closes one of those four gaps without opening a fifth: it
gives the model **your context**, a **reviewed way to act**, and the **armour**
that makes handing it that much private information a reasonable thing to do.

## The problem it is pointed at

Academic work carries a long tail of small administrative tasks that are
tedious rather than difficult: answering the fourth email this week asking for a
meeting, working out whether you can actually take on a review, writing the
letter you promised in March, finding the slot that works for five people,
chasing the form somebody needs back by Friday. None of it is hard. All of it is
constant, and it arrives faster than it leaves.

What stops a language model from absorbing that work is not intelligence. It is
that every one of those tasks needs **your** context — who this person is, what
you already promised them, what is actually on your calendar, how you write when
you say no. A model with no context can only produce something generic that you
then have to rewrite, which is slower than doing it yourself.

So the assistant that would actually help is one that can see a great deal about
you. And that is precisely the assistant it is most dangerous to build.

## What it plugs into

Context is the whole differentiator, so it is worth being concrete about where
it comes from. None of this is required — mecha is useful with none of it — and
each is wired in separately.

| | What it gives the model | How |
|---|---|---|
| **Mail and calendar** | Gmail and Outlook behind one surface. The model names an *account*, never a provider, and reads fan out across every mailbox. | [`mecha-mail`](/docs/features/mail) |
| **Documents** | Google Docs, Sheets and Slides — but only files it created or you handed it in Google's own picker. | [`mecha-docs`](/docs/features/documents) |
| **A knowledge graph** | Who people are, what happened when, what you already promised. Fed by ambient conversation capture (Bee), a calendar feed, Slack, messages and mail exports. | [Memory](/docs/features/memory) |
| **Slack** | A remote control: watch a run from a phone, approve what it wants to send, pass files both ways. | [Slack](/docs/features/slack) |
| **Anything else** | Connecting a new source is configuration, not a code change. | [MCP](/docs/features/tools-and-mcp) |

The knowledge graph is the piece that makes the rest add up. Mail and calendar
tell the model what is *happening*; the graph is what lets it know who these
people are to you and what you said last time.

## Three things that make this safe enough to do

An assistant worth having is one you have handed your mail, your calendar and
your memory. Three properties are what make that a reasonable trade rather than
a reckless one, and each is structural rather than promised.

**The model is local.** Not a fallback for when the API budget runs out — the
target. Your mail is read by weights on your own machine, and the data has no
occasion to leave it. What that costs in hardware is a shorter answer than
people expect: see [Choosing hardware](/docs/getting-started/hardware). That choice also shapes the engineering: the binding
constraint on a small model in a loop is tool-call reliability rather than
intelligence, which is why [the eval rig](/docs/features/evaluation) grades the
tool-call trace before the prose.

**The memory is encrypted at rest.** The knowledge graph is
**SQLCipher-encrypted**, with the key resolved from an environment variable, a
keyfile, or a local file at mode 0600 — and the keyfile is meant to be backed
up separately from the database, because without it the graph is unrecoverable
and with it alone an attacker still needs the file.

**The lethal trifecta is refused, not discouraged.** Which is the next section,
because it is the decision the rest of the design hangs off.

## Why the security model is the centre of the design

An agent that holds three things at once can be turned against you:

1. **Private data** — your mail, your calendar, your notes, your knowledge graph.
2. **Untrusted content** — anything written by someone else. An email body. A
   web page. A calendar invite's title. A PDF a stranger sent.
3. **A way to send** — replying, posting, publishing, or merely fetching a URL,
   since a payload fits in a query string.

This is Simon Willison's **lethal trifecta**, and the uncomfortable part is that
a personal assistant has all three *by definition*. Reading your mail is what it
is for; the mail was written by other people; answering it is the point. You
cannot design the trifecta out of the job. You can only decide what happens when
all three are present.

Most harnesses handle this by telling the model to be careful. That does not
work, because the injected instruction arrives through exactly the same channel
as the legitimate data, and the model has no way to tell them apart. mecha
treats it as a property of the system rather than a matter of the model's
judgement: **every tool declares what it can do, the conversation tracks what
has entered it, and an outbound call is refused once both private data and
third-party content are present.** The refusal happens before the human is asked,
because a person clicking "yes" is what an injection is trying to engineer.

That single decision shapes most of the rest of this documentation — the path
jail, the sandbox, the outbox, the front door, subagent isolation and the
provenance rules on learning are all consequences of taking it seriously. See
[Security model](/docs/features/security) for how it is enforced.

## The anatomy of the suit

```
                    ┌─────────────────────────────────────────┐
   PERSONAL         │              mecha-core                 │      THE WORLD
   CONTEXT          │                                         │
 ┌────────────┐     │   the loop · tools · MCP client          │   ┌──────────────┐
 │ knowledge  │     │   taint tracking · path jail             │   │ mecha-factory│
 │ mecha-graph│────▶│   sandbox · budgets · compaction         │──▶│ published    │
 │ mail       │     │   sessions · learning · triggers         │   │ artifacts    │
 │ calendar   │     │                                          │   │              │
 │ files      │     │            ▲              │              │◀──│ typed        │
 └────────────┘     │            │   outbox     ▼              │   │ requests in  │
                    │        ┌───┴──────────────────┐          │   └──────────────┘
                    │        │  you, reviewing      │          │
                    │        └──────────────────────┘          │
                    └─────────────────────────────────────────┘
```

**The frame — [`mecha-core`](/docs/features/interfaces).** The loop that sits
between a model and the things it can do: ask the model, run the tools it asks
for, feed the results back, repeat until there is an answer. Around it sit the
things a loop needs to survive contact with real work — a tool registry, an MCP
client, transcripts, budgets, retry classification, and compaction so a long
conversation does not simply stop being sendable. It is a plain Rust library
that knows nothing about any application; the `mecha` binary is a thin layer on
top of it.

**The senses — personal context.** An assistant is only as good as what it
knows about you, so mecha is built to be wired into a lot of it. Mail and
calendar arrive through [`mecha-mail`](/docs/features/mail), which puts every
account behind one surface so the model names an *account* (`dartmouth`,
`personal`) and never a provider. A [personalized knowledge
graph](/docs/features/distillation) supplies who people are, what happened
when, and what was said — and mecha feeds it back, distilling each closed
session into an episode. Everything else comes over MCP, which is the seam that
keeps this open-ended: connecting a new source of personal context is
configuration, not a code change.

**The hands — [`mecha-factory`](/docs/factory/overview).** An assistant that can
only talk to you in a terminal is not much of an assistant. The factory is the
public surface in both directions: what the agent makes becomes a durable,
versioned, permissioned URL you can read on a phone or send to a collaborator,
and what other people need from you comes back as a **typed request** rather than
free-form prose. One request type emits the web form, the JSON Schema, and the
MCP tool at once, so a human with a browser and another agent with a tool call
both arrive at the same typed object.

**The pilot — you.** Anything the agent would send passes through
[the outbox](/docs/features/outbox) first: tools you name are *staged as drafts*
rather than executed, so overnight inbox triage leaves you a review queue
instead of sent mail. This is a property of the harness, not of the email tool,
which means a third-party MCP server is covered by it without knowing it exists.

## What makes mecha different

Plenty of agent harnesses exist. These are the choices that are actually
unusual, rather than the ones everybody makes.

**Local-first changes what the engineering is about.** Beyond the privacy
argument above: a model that is five percent smarter but malforms its arguments
one call in twenty is *worse* in a loop, because every bad call costs a recovery
turn. It is also why context accounting is explicit — nothing in any provider's
API reports how much context is *left*, so mecha is told the window and derives
its compaction threshold, its per-turn tool-output budget and its gauge from it.
Which is exactly the number people get wrong, and why
[`mecha setup`](/docs/getting-started/setting-up) reads it back off the server
instead of asking you.

**Security is structural, not prompted.** The trifecta interlock lives in the
type system and the loop, not in the system prompt. Taint is a property of the
**conversation**, so a new turn does not launder it — fetch a hostile page on
turn one and read a secret on turn two, and the interlock still sees both. Path
containment is a function every tool must call, not a rule tools are asked to
follow. A configured sandbox that cannot actually confine anything **stops the
run** rather than falling back to running unconfined, because a security control
that degrades quietly is worse than one that was never there.

**Sending is staged by default, and reviewed by a person.** The interesting
consequence is that the *useful* configuration and the *safe* configuration are
the same one. An unattended overnight run that drafts nine replies needs no
write permission at all, because staging executes nothing.

**It expects to run unattended.** [Triggers](/docs/features/triggers) put a
prompt on a cron schedule; a missed week owes one briefing rather than seven;
each run is jailed to [its own work directory](/docs/features/work), which is
also where its output durably lands, so yesterday's briefing is an ordinary file
in today's run. A scheduled run gets no additional trust — the same interlock,
jail, sandbox and budgets apply, and it deliberately cannot read a project's
`mecha.toml`, because a cloned repository must not be able to shape a job on
your machine.

**What it learns has to keep earning its place.** mecha mines the moments you
stepped in — a mid-run steer, a denied tool call, a corrective follow-up — and
consolidates them into [rules](/docs/features/learning) that ride in the system
prompt. Two guards make that safe rather than merely clever. Rules are gated on
**provenance**: a lesson drawn from a conversation that had read untrusted
content is excluded structurally, because a learned rule is a longer-lived
injection path than anything the interlock guards. And rules are gated on
**measurement**: a validation ledger records whether each rule actually changed
an answer, and one that accumulates attributed regressions is proposed for
retirement. Measured harm, not a model's confidence in itself.

**Everything a model says about its own work is treated as hearsay.** Runs are
recorded as append-only transcripts and can be [replayed against today's
code](/docs/features/sessions-and-replay). Eval cases can end in a `verify`
command whose exit status is the grade — not whether the model reported the
tests passing, but whether they pass. Repeated runs report **pass^k** beside
pass@k, because reliability decays much faster than mean success and a
single-run scorecard cannot tell a flaky case from a solid one.

**And the harness measures itself.** Every finished run records how it went, not
only what it cost, so a run that quietly failed a third of its tool calls is
visible instead of silent. `mecha doctor` reads those
[populations](/docs/features/run-quality), `mecha diagnose` proposes one change
with a falsifiable prediction, and `mecha eval --ab-config` is the measurement
that would refute it — paired by case, confirmed on a holdout, and rejected
outright if the gain was bought by attempting less work. Nothing in that loop
applies itself.

## Where to go next

- [Installation](/docs/getting-started/installation) — build it from source.
- [Choosing hardware](/docs/getting-started/hardware) — what memory actually buys
  you, and recommended configurations by tier.
- [Setting up](/docs/getting-started/setting-up) — point it at a model, and let
  `mecha setup` read the settings back off the server rather than typing them.
- [First run](/docs/getting-started/first-run) — one-shot, a REPL, and
  full-screen.
- [Configuration](/docs/getting-started/configuration) — the layered TOML, and
  the settings that matter early.
- [Design principles](/docs/principles) — the rules the code keeps, and what
  each one cost to learn.
- [Security model](/docs/features/security) — read this before giving an agent
  anything private.
- [The factory](/docs/factory/overview) — publishing out, and typed requests in.
- [CLI reference](/docs/reference/cli) — every command and flag.
