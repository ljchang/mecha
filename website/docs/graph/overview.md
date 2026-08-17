---
title: Overview
sidebar_position: 1
description: What mecha-graph is, how to install it, and how it becomes mecha's memory.
---

# mecha-graph

A personal knowledge graph that turns your own data — mail, calendar, notes,
messages — into context any agent can use. It is mecha's
[memory](/docs/features/memory), and it is deliberately its own project:
[github.com/ljchang/mecha-graph](https://github.com/ljchang/mecha-graph),
three crates on crates.io, served over MCP to any client, usable without
mecha at all.

**The deliverable is not a database, it's a context pack**: every interface
returns a token-bounded, provenance-carrying, freshness-stamped slice.

Data imports as **episodes** (append-only evidence, idempotent by source
id); linkers wire episodes to **entities** through mentions; and **facts** —
bi-temporal interpreted claims, each with episode provenance — are either
asserted directly by high-trust sources or staged as candidates that your
review promotes. *Episodes are evidence, nodes are things, facts are
beliefs, the context pack is the product.* The full mental model is in
[Architecture](./architecture).

## Install

```bash
cargo install mecha-graph          # the CLI
cargo install mecha-graph-mcp      # the MCP server
```

Embeddings use [ollama](https://ollama.com) with `nomic-embed-text` on
localhost; everything else is self-contained. To see it work with no
personal data at all, a checkout's `eval/synthetic/run.sh` builds a
throwaway graph from a fictional corpus and grades 24 retrieval queries
against it.

## Feed it

```bash
mecha-graph source add ics --url '<secret-ical-url>' --me you@example.edu
mecha-graph source add mbox --path ~/Takeout/mail.mbox --me you@example.edu --retention capture_delete
mecha-graph source sync            # cursored, idempotent — re-runs are no-ops
mecha-graph link --auto
mecha-graph embed
mecha-graph query "what did we discuss about the pilot data?"
```

Per-source auth and configuration live in [Integrations](./integrations).
The store is SQLCipher-encrypted at `~/.mecha-graph/graph.db`, with the key
beside it (mode 0600 — back it up separately); sends nothing anywhere, and
`redact` is a true delete. The full privacy story is in the
[repository README](https://github.com/ljchang/mecha-graph#your-data-stays-yours).

## Wire it into mecha

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"
# The kg_* tools carry their own namespace; skip the graph__ prefix.
prefix_tools = false

# The graph holds other people's words, so reading it must arm the
# trifecta interlock. No MCP annotation can declare that; config forces it.
[mcp.capabilities]
untrusted_input = true
```

Why the override matters — and how episodes, corrections, and review move
between the two projects — is the [Memory](/docs/features/memory) page's
story. Any other MCP client wires the same binary with none of this:
`claude mcp add graph -- mecha-graph-mcp`.

## How it improves itself

The graph is designed to get better with minimal oversight: an autonomy
ladder for extracted claims, a mechanical error contract for corrections,
and adversarial "gossip" sessions that surface gaps and contradictions —
the whole design, with its settled decisions and build order, is in
[Self-improvement](./self-improvement).
