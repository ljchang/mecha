---
title: The front door
sidebar_position: 13
description: Inbound requests from strangers, and the quarantine that means a privileged run sees the extraction and never the prose.
---

# The front door

Requests from the outside world arrive in `~/.mecha/requests/` as JSON, drained
from [the public surface](https://github.com/ljchang/mecha-factory) by a
process that holds the drain key and nothing else. `mecha frontdoor` is
everything that happens to them afterwards, and the whole of it exists to serve
one sentence:

> **The privileged run sees the extraction, never the prose.**

```bash
mecha frontdoor                    # what has arrived, and each request's state
mecha frontdoor list --state extraction_failed
mecha frontdoor show 42            # the full request, including what a stranger wrote
mecha frontdoor extract            # run the quarantined pass over anything new
mecha frontdoor next --limit 5     # what a triage run may be told, as JSON
mecha frontdoor triage --limit 5   # draft a reply to each extracted request
mecha frontdoor needs-info 42 --note "which week?"
mecha frontdoor close 42 --reason "answered in person"
```

## Why a quarantine

A run holding the calendar and the mailbox is the most dangerous context in
this system, and a free-text field is the one place a stranger controls the
bytes.

The typed form is already doing most of the work. Nothing anyone types can
change what *kind* of request theirs is, or its priority, or whether consent
exists, because those are enums and booleans the origin validated. What remains
is prose — and prose is where an instruction can hide.

So the shape is CaMeL's dual-LLM split, at a size where it is cheap:

```text
  free text ──▶ extractor (no tools, no history, JSON only)
                    │
                    ▼
                typed fields ──▶ triage run (calendar, mail, drafts a reply)
                    │
  free text ────────┴──▶ shown to you, never to the privileged pass
```

## The verbs, and the split between them

- **`list` and `show` are for you.** `show` prints the prose, because a person
  reading a stranger's request in a terminal is the safe context: you cannot be
  prompt-injected into sending your own calendar somewhere.
- **`extract` is the quarantined pass.** A tool-less model call per record,
  turning prose into typed fields. Nothing it produces has any authority; it is
  the *only* representation of the prose a privileged run will ever see.
- **`next` is what a triage trigger runs.** It prints exactly what the boundary
  allows and nothing else, so the thing feeding a run with calendar and mail
  access cannot accidentally include the words a stranger typed.

Draining is deliberately *not* here. `mecha-factory-publish drain` speaks the
protocol and holds the key, and the common case — nothing new — has to cost
zero tokens and no model at all.

## And a request has to be able to reach an answer

Otherwise the queue only grows, which is the failure this component exists to
fix. Three verbs end one:

- **`triage` is the privileged half.** A full agent, with mail and calendar,
  told only what `next` would print, ending in outbox drafts and never in mail
  in flight. Each request gets its own conversation, so prose flagged as reading
  like instructions cannot arm the [interlock](/docs/features/security) for the
  request behind it.

  **It refuses to run without the outbox route**, rather than running unrouted:
  without it a `mail_send` the model makes actually sends, and a stranger's
  inbox is not where you want to discover that `[outbox] tools` was unset.
- **`needs-info` parks a request** until the requester answers something.
  `--note` records what is missing, and it replaces the previous note even when
  absent — a stale explanation attached to a new state reads as an explanation
  of that state.
- **`close` ends one, and `--reason` is required.** Not optional: silence is
  precisely the failure mode this component exists to fix, and a request that
  went away without a recorded reason is indistinguishable from one that was
  dropped.

The join between a request and its drafts needed no building. A staged outbox
item already records the session that drafted it, so a triage run with its own
session is enough to say which drafts belong to which request. `reconcile` reads
the outbox and updates the request store, and it runs on `list` and `next` on
its own rather than on a verb you have to remember: a state that is only correct
after someone runs a command is a state nobody can trust. The outbox has still
never heard of a request, and `mecha outbox send` — another process, hours
later — closes the loop without knowing it is doing so.

Three decisions there:

- **A rejected draft returns the request to `extracted`, never to `closed`.**
  "Not this reply" is not "not this request", and a request closed because its
  first draft was wrong is exactly the silence this exists to fix. The rejection
  reason rides along, and the request becomes a triage candidate again.
- **A partly-resolved set is left alone.** Some sent and some pending is a
  person mid-review, not a state to settle on their behalf. So is a request
  whose drafts have been swept: unknown stays unknown and waits for a person.
- **Reconciliation is best-effort.** No outbox is a perfectly ordinary machine,
  and a `list` that refuses to print because a cross-check store is absent would
  be worse than one printing slightly stale states.

## Five decisions, each a bug if undone

**The boundary is a function, not a rule.** `Record::for_privileged_run`
returns the non-prose values plus the extraction, and there is deliberately no
argument that makes it return the prose. A caller that wants the original is a
human running `frontdoor show`. If this were "remember not to include the free
text", it would hold until the first person in a hurry.

**Which fields are prose is not decided here.** The drain writes `free_text`
onto the record from the manifest, where free-text-ness is derived from the
field kind. Guessing at it on this side — by looking for long strings, say —
would be exactly the mistake of letting the caller be wrong about which values
are dangerous.

**An extraction failure is not a silent pass-through.** The record goes to
`extraction_failed` and waits for a human. It never falls back to handing the
prose on, which is the one behaviour that would make the whole layer
decorative.

**The extractor gets no tools and no conversation.** Not "is told not to use
tools" — is issued a request with an empty tool list and a single user message.
There is nothing for an injected instruction to reach.

**Reasoning comes first in the output, the typed fields after.** Constrained
decoding degrades reasoning when the answer precedes the thinking, and this is
the one call in the system whose output is trusted downstream by construction.

## States

```text
drained ──▶ extracted ──▶ triaged ──▶ awaiting_me ──▶ answered
   │         ▲                             │
   │         └───── every draft rejected ──┘
   │
   ├──▶ extraction_failed     (at any point; routes to a human)
   └──▶ needs_info            (parked until the requester answers)

  any state ──▶ closed        (always with a reason)
```

`triage` moves a request to `triaged`, and `awaiting_me` is where it sits while
its drafts wait in the outbox. Releasing one gets `answered`; rejecting all of
them gets `extracted` again, with the rejection reason attached.

A record that did not validate against the manifest at drain time is never
extracted and never reaches a run.

## The seam is a directory of JSON

Records are deserialised structurally rather than through a shared type: the
boundary between the public surface's client and mecha is a directory of files,
not a crate dependency. Unknown fields are preserved on write, because the
writer on the other side may know things this one does not.

## Where to go next

- [Security model](/docs/features/security) — the interlock, and why a second layer was still worth building.
- [Publishing](/docs/features/publishing) — the outbound half of the same boundary.
- [Triggers](/docs/features/triggers) — what a triage run is scheduled by.
