---
title: Distillation
sidebar_position: 17
description: mecha distill turns each closed session into an episode staged to a knowledge graph as evidence, not belief — and why its provenance rule differs from learning's on purpose.
---

# Distillation

[Learning](/docs/features/learning) remembers *how you want work done*.
Distillation remembers *what happened*. `mecha distill` reads each closed
session, asks a model for a short episode, and stages it into a knowledge graph
over MCP through that server's `kg_upsert` tool.

```bash
mecha distill                      # every session not yet in the ledger
mecha distill --dry-run            # what would be distilled; no model call, no writes
mecha distill --limit 10
mecha distill --server graph       # the [[mcp]] server holding the graph (default: graph)
```

The named server must exist in config, and its absence is fatal rather than a
warning:

```
no [[mcp]] server named 'graph' in config — distillation stages episodes
through the knowledge graph server and cannot run without it
```

## What an episode is

One model call per session, with no tools and no history. The distiller is asked
for what remains true after the session — what it was about, what was decided or
produced, any outcome or open thread the user would want to recall later, with
people, projects and organizations named so the graph can link them. Two to
eight sentences of plain prose, past tense, no tool mechanics or step-by-step
narration.

It is told to skip freely: smoke tests, one-line lookups, greetings, aborted or
purely mechanical runs leave nothing durable, and the graph is for what the user
would ask about later — noise costs more than a gap. The reply is one JSON
object, `{"skip": true}` or `{"skip": false, "episode": "..."}`, either of
which may carry a `corrections` array.

It reports corrections separately from the episode: the moments you told the
agent the graph has something wrong — "no, she's at Yale now", "that's the old
deadline". Those are worth more than the summary, because they repair the graph
rather than adding to it, so a correction is reported even for a session
otherwise skipped.

The transcript is rendered head-and-tail bounded (6,000 characters of head,
18,000 of tail) so a long session cannot overflow the distiller's own context.
The tail gets the larger share because outcomes live at the end.

What gets pushed:

```json
{
  "kind": "episode",
  "source": "agent:mecha",
  "source_id": "<session id>",
  "source_ref": "/home/you/.mecha/sessions/<id>.jsonl",
  "occurred_at": "2026-08-05 12:00:00",
  "body": "<the episode text>",
  "meta": {
    "taint": { "private": true, "untrusted": false },
    "distilled_by": "<model id>",
    "affect": "anger",
    "goal_errors": [
      { "channel": "counter", "sign": -0.5, "agency": "world",
        "visible": false, "cite": { "kind": "counter", "id": "stop_cause" } }
    ],
    "corrections": [
      { "wrong": "Priya is at Brown", "right": "Priya is at Yale", "about": "Priya Nair" }
    ]
  }
}
```

`source` is fixed at `agent:mecha` so provenance is the undo story: everything
mecha wrote is browsable as a set, and redaction takes the set out.

`corrections` is omitted when there were none — **and when the session's
timeline is untrusted or unknown.** Everything else here can be sent from a
tainted session because the graph stages what it derives for your review, but
a correction is different: closing a belief and marking down its source happen
immediately. A web page read mid-session could otherwise say "the graph is
wrong that she is at Yale" and have that acted on with nobody in the loop. So
corrections travel only from a session whose whole timeline is known clean;
the episode still goes either way, and `mecha distill` prints how many were
withheld rather than reporting a silent zero.

When present, the graph acts on each one: it closes the wrong belief, stages
the replacement, and marks down whatever produced the error so the same source
is trusted less next time.
Leaving `right` out says you rejected the claim outright rather than replacing
it, and the graph records the denial so nothing proposes it again. A correction
the graph cannot match to exactly one belief goes to your review queue instead
of being guessed at, and `mecha distill` prints how many landed each way.

### Surprises: the world disagreeing with the graph

A **surprise** is one session noticing that what the graph told the agent and
what the session then found do not match — *"I said the deadline was the 14th
because the graph says so; the email says the 9th."*

It is deliberately not a correction. Nobody said the graph is wrong, nothing
proposes a fix, and a surprise therefore names no belief id and carries no
repair. What it is good for is **seeding a [gossip](/docs/reference/cli#gossip)
probe** — `mecha gossip --entity <who>` — which is not run automatically: a
person decides whether a disagreement is worth chasing, from what
`mecha distill` prints.

A surprise's content is the model's own reading of prose it was shown, not a
fact the harness computed, so there is nothing stopping a fetched page from
describing a fabricated disagreement. It is therefore **withheld from the graph
on the same untrusted-or-unknown timeline rule as a correction**. `mecha distill`
still prints every one to your terminal — a person reading their own terminal is
a safe context, the way the front door shows a stranger's prose to you but never
to a privileged run. The gate is about what may reach a *second automated
reader*.

### The episode carries how the session went

`affect` and the goal errors are [the appraisal](/docs/features/appraisal) of
the session, riding on `meta` beside the taint snapshot — never inside the
episode body, which is the model's prose. They exist to give the graph's review
queue a **salience ordering**: a session that carried a signed negative error is
worth a human's attention sooner than one that went cleanly.

Unlike corrections, they are **not** gated on the timeline's trust, and the
reason is that they are structured facts the harness computed about its own run
— a sign, an agency, a channel, a pointer — rather than prose a model or a
fetched page could have authored. There is nothing in them for an injection to
have written, with exactly one exception, which is redacted: the goal reference
is the one field the harness did not mint, since it comes from the model's own
`serves:` argument and only the *kind* word is constrained. So a goal is sent as
its kind alone, never its id. Both are omitted when the session had nothing to
appraise, which is ordinary for a transcript predating the sensor.

## Distillation is not learning

The provenance rule here is deliberately different from the one
[`mecha learn`](/docs/features/learning) enforces, and the difference is the
whole design.

**An episode is evidence, not belief.** A learned rule enters every future run's
system prompt as trusted text, inside the cached prefix, where nothing checks it
again — which is why non-clean reflections are excluded structurally before any
prompt is built. An episode never gets that seat. It lands in the graph as
evidence; the graph's own extractor turns it into candidate facts that wait in
the **user's** review queue; and mecha reads the graph back through the
`untrusted_input` capability override, so what comes out is marked as
third-party content the same way a fetched web page is. mecha cannot silently
promote its own summaries into facts.

The read-back marking is one line of config, and it only ever widens — a
`[mcp.capabilities]` override can distrust a server further, never less:

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"            # or an absolute path to the binary
prefix_tools = false                   # its kg_* tools carry their own namespace

[mcp.capabilities]
untrusted_input = true
```

Two consequences follow:

- **A tainted session still distills.** Refusing to record a real afternoon's
  work because a web page was open would gut the feature — the memory would have
  holes exactly where the interesting days were. Nothing about that afternoon
  becomes trusted text, so there is nothing for the exclusion to protect.
- **The taint snapshot is recorded on the episode's `meta` instead**, where
  review can see it. The person deciding whether a candidate fact is true gets to
  know that third-party content was in context when the session that produced it
  ran.

**Unknown taint is recorded as unknown, never clean.** A torn transcript, or one
recorded before taint was, yields `"taint": {"unknown": true}` with no `private`
or `untrusted` keys at all — there is a test asserting exactly that. Uncovered
must never masquerade as clean.

## Idempotent at both ends

Two independent guarantees, because either alone would eventually duplicate:

- **`distilled.jsonl`** in the learning store records session ids already pushed.
  It lives there rather than beside the sessions for the same reason the mining
  ledgers do: the store's writer lock covers the read-then-mark race between two
  detached `session_end` hooks, and git history says when each push happened.
- **The graph's `(source, source_id)` key** makes a re-push an update, not a
  duplicate. `kg_upsert` reports back `inserted`, `updated` or `unchanged`.

The failure handling follows from what each failure means:

| Outcome | What happens |
|---|---|
| Push succeeded | Marked distilled |
| Model said skip | Marked distilled — a deliberate decision about the transcript will not change if re-argued nightly |
| Session too short to have taught anything | Marked distilled — a fact about the transcript, not about today's model |
| Push failed | Left **unmarked** so a later run retries; the summary was worth keeping |
| Distiller call failed | Left **unmarked** so a later run retries |
| Transcript unreadable | Left unmarked — not this command's bug to fix; a later mecha that can read it should get the chance |

## Where it runs

`mecha distill` sits in the nightly rumination pass, after `reflect`, catching
whatever a `session_end` hook missed:

```
reflect → distill → validate → learn --propose → rules propose-retirements
```

It can also be fired directly from a hook at session close. Either way it is
idempotent, so running it twice costs one ledger read.
