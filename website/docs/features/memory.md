---
title: Memory
sidebar_position: 13
description: Three memories with three trust models — the knowledge graph for the world, the learning store for behaviour, the transcript for what was actually said.
---

# Memory

mecha deliberately has **three memories**, because three different kinds of
thing are worth remembering and no single trust model fits them all:

| Memory | Holds | Trust model | Write path |
|---|---|---|---|
| **The knowledge graph** | The world: people, projects, events, facts | **Untrusted on read** — third-party text by construction | Agent writes are staged candidates; a person promotes them |
| **The [learning store](/docs/features/learning)** | Behaviour: rules mined from your corrections | Provenance-gated — only clean-origin reflections become rules | `reflect` → nightly consolidation → gated proposals |
| **The [session transcript](/docs/features/sessions-and-replay)** | What was actually said and done, verbatim | The record itself; `recall` re-surfaces it taint-neutrally | Append-only, rewrites recorded with what they replaced |

The separations are load-bearing. A learned rule rides in every future
prompt's cached prefix — a longer-lived injection path than anything else in
the system — so learning is gated on provenance and never fed by machine
policy. The graph is fed by mail, messages, and pages other people wrote, so
reading it must arm the trifecta interlock. The transcript is your own
record, so re-reading it can be free. Collapse any two of these and one
trust model has to lie.

## The knowledge graph: mecha-graph

The world-memory is its own project —
[**mecha-graph**](https://github.com/ljchang/mecha-graph), a personal
knowledge graph served over MCP to any client, published on crates.io and
usable without mecha. Your mail, calendar, notes, and messages import as
**episodes** (append-only evidence); linkers wire them to **entities**; and
**facts** — bi-temporal claims, each carrying its evidence — are what
retrieval builds context packs from. It has its own
[section of this site](/docs/graph/overview) — architecture, integrations,
the self-improvement design, and its changelog, synced from the repository
at every build; this page covers the mecha side of the seam.

### Wiring

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"      # cargo install mecha-graph-mcp
# The kg_* tools carry their own namespace, so skip the graph__ prefix.
prefix_tools = false

# The graph holds other people's words — mail bodies, invite titles,
# extracted claims — so reading it must count as untrusted input. No MCP
# annotation can declare that, so config forces it, and the override can
# only ever widen.
[mcp.capabilities]
untrusted_input = true
```

The tools arrive as bare `kg_search`, `kg_entity`, `kg_timeline`,
`kg_related`, `kg_upsert`, and the verification family (`kg_verify`,
`kg_pending`, `kg_verdict`, tasks).

### Why reading memory arms the interlock

A personal graph fed by mail holds third-party text by construction, and
text from outside can carry instructions. Marking the server
`untrusted_input` is what makes the [trifecta
interlock](/docs/features/security) refuse an outbound call once memory and
private data are both in the conversation — retrieving a memory and then
reaching for `http_fetch` is exactly the exfiltration shape the interlock
exists to stop. The consequence is real and intended; relax it with
`[security] trifecta = "ask"` if you would rather be prompted than stopped.

### How memories get in

Nothing an agent writes becomes a belief without review. Two paths feed the
graph from mecha's side:

- **[Distillation](/docs/features/distillation)** — `mecha distill`
  summarises each closed session into an episode and stages it through
  `kg_upsert`. The graph's extractor turns episodes into *candidates* that
  wait in your review queue; a tainted session still distills (losing the
  record of a real afternoon because a web page was open would gut the
  memory), with the taint recorded on the episode for review to see.
- **Corrections** — when you tell the assistant the graph is wrong, the
  distiller ships the correction and the graph acts: supersede the wrong
  belief, stage the replacement, demote whatever produced the error. The
  one exception to "tainted sessions still distill": corrections from
  untrusted transcripts are withheld entirely, because a fetched page must
  not be able to evict a true belief with nobody in the loop.

Reads fan the other way — `distill`, `vet`, and `corroborate` default to
`--server graph`, and the graph's own review tooling (`mecha-graph review`,
`accept`, `reject`, `precheck`) is where staged candidates become beliefs.

## What memory is *not*

The graph never enters a prompt as trusted text, rules never come from
machine refusals, and the transcript search can only reach the conversation
it records. Each "never" has a test or a structural guard behind it — the
details live in [Security](/docs/features/security),
[Learning](/docs/features/learning), and
[Sessions and replay](/docs/features/sessions-and-replay).
