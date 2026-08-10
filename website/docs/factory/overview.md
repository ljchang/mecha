---
title: What the factory is
sidebar_position: 1
description: mecha-factory is the public surface in both directions — durable URLs for what an agent makes, and typed requests for what the world needs from you.
---

# What the factory is

An assistant that can only talk to you in a terminal is not much of an
assistant. Real work has to be shown to people, and real work arrives from
people. **mecha-factory** is the surface for both, and it is deliberately not
part of the machine:

> A factory is where machines are built and shipped from — and it is
> deliberately *not* the machine. Orders come in, product goes out.

It lives in [its own repository](https://github.com/ljchang/mecha-factory) and
has no dependency on `mecha-core`. The contract between them is data.

## Two directions of one boundary

**Outbound — publish what mecha makes.** A report, a dashboard, a morning
briefing, an interactive notebook: rendered into a bundle, versioned
immutably, and served at a durable permissioned URL you can open on a phone,
send to a collaborator, or have a later agent run read back. See
[Artifacts](/docs/factory/artifacts).

**Inbound — build interfaces back into mecha.** Someone wants to invite you to
speak, apply to the lab, request a letter, or book a meeting. Instead of that
arriving as yet another email for you to parse, it arrives as a **typed
object**. One request-type manifest emits the HTML form, the JSON Schema, and
the validator both ends run — so a human with a browser, an agent with a
browser, and an agent with MCP all arrive at the same typed result.

A form is the **default rendering, not the point**. Adding a modality is a
renderer, not a parallel system, which is why the same machinery already
produces booking pages and [polls](/docs/factory/polls) without either being a
special case.

## Why this is the safe shape

The inbound direction is the one that matters for security, and it is the
reason the factory exists as a separate thing rather than as a webhook into
your agent.

A privileged run — one holding your mail, your calendar and your knowledge
graph — is the most dangerous context in the system, and a free-text field is
the one place a stranger controls the bytes. The typed form already does most
of the work: nothing anyone types can change what *kind* of request theirs is,
its priority, or whether consent exists, because those are enums and booleans
the origin validated.

What remains is prose, and prose is where an instruction can hide. So:

**The server filters shape; only mecha filters meaning.** The box checks that a
submission is well-formed against the schema. Whether a well-formed field is an
attempt at prompt injection is a judgement, and that judgement happens at home,
in [the front door](/docs/features/frontdoor), where free text goes to an
extractor with no tools and no history before any privileged run sees it.

**Assume the public box is lost.** It holds no provider key, no model, and
nothing that reaches home. Every key is stored as an Argon2id hash. Packets go
one way — home publishes and drains; the box never dials home. Losing the box
costs you the box.

## What is here

| Crate | What it is |
|---|---|
| `mecha-manifest` | The versioned data contract. Request types, bundles, their JSON Schema, their HTML form, and the one validator both ends run. Pure — no I/O, no network. |
| `mecha-factory-publish` | The home side. Renders and versions bundles, moves the alias a share URL resolves through, holds the publish key, and serves the MCP surface that mecha wires. |
| `mecha-factory` | The box. One binary serving three origins under three policies, an authenticated write API, and a queue home drains. Multi-user: every row belongs to a person. |

The split is the security model made structural — the crate that holds the
credential and the crate that faces the public are not the same crate, and the
public one has nothing worth stealing.

## Three origins

The deployed box serves three names, and which origin may serve a bundle is
decided by one function of the bundle's class rather than by configuration:

| Role | URL | Serves |
|---|---|---|
| gate | `gate.mecha-factory.ai` | forms, booking, polls, the signed-in viewer, the API |
| artifacts | `<handle>.art.mecha-factory.ai` | static and interactive bundles |
| compute | `<handle>.compute.mecha-factory.ai` | notebooks, under a policy that permits WASM |

They are separate origins because they run under genuinely different content
policies, and a browser's origin is the only boundary that actually enforces
that. Forms are path-scoped on the gate rather than given their own origin,
deliberately: server-rendered HTML with no script executes nothing, so there is
nothing for an origin to separate.

A publish answers with **two URLs that are not interchangeable** — a viewer page
for a person, carrying the version menu and owner controls, and a bare bytes URL
for a machine. Quote the one the tool gave you rather than composing one.

## How a stranger's request reaches you

```
  stranger ──▶ POST /f/<handle>/<type>      validated against the schema
                      │                      you uploaded earlier
                      ▼
              verification email            unverified never enters the queue
                      │
                      ▼  click
                  queued on the box
                      │
                      ▼  factory-publish drain          (at home, on a timer)
              ~/.mecha/requests/
                      │
                      ▼  mecha frontdoor extract        (no tools, no history)
              typed extraction ──▶ mecha frontdoor triage ──▶ outbox draft
                                                                   │
                                                                   ▼
                                                          you, reviewing
```

Two properties of that pipeline are worth stating plainly, because both are
easy to undo:

- **Unverified submissions never enter the queue.** The verification token is
  single-use and stored as a hash, and mail is budgeted per recipient.
- **`drain` is a CLI command and deliberately never an MCP tool.** The common
  case is "nothing new", which has to cost zero tokens and no model at all — a
  timer drains, and only spawns an agent when something actually arrived. It
  also means a stranger's prose is never fetched *into* a context that already
  holds tools.

## How mecha talks to it

`factory-publish` is both the CLI and the MCP server; there is no daemon.

```toml
[[mcp]]
name = "factory"
command = "factory-publish"
args = ["mcp"]
sandbox = true

[outbox]
tools         = ["factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish"]
publish_tools = ["factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish"]
```

Because [the outbox](/docs/features/outbox) routes by tool *name*, naming these
stages them for review with no change to mecha at all. `bundle_render` is
deliberately not routed: rendering is cheap and local, and making every
iteration cost a human review is how a review queue stops being read.
`publish_tools` additionally tells the review surface that these items are
publishes rather than prose, so `show` leads with the rendered page instead of
with a path and a visibility flag.

See [Publishing](/docs/features/publishing) for the full tool surface, and
[Onboarding](/docs/factory/onboarding) for pairing a machine to a handle.

:::warning[One honest gap]
The notebook renderer executes code that mecha did not write — `marimo export`
runs the notebook to capture its state — and **that subprocess is not yet
confined**. It runs as you, with your environment. mecha confines the MCP server
it launches, but the render subprocess lives inside the factory crate, where
mecha's sandbox cannot see it. Do not wire notebook rendering to anything
unattended until it is confined and preflighted.
:::

## Where to go next

- [Onboarding](/docs/factory/onboarding) — claim a handle and pair a machine.
- [Artifacts](/docs/factory/artifacts) — versions, aliases, visibility, sharing.
- [The component gallery](/docs/factory/gallery) — every field kind and state,
  rendered by the renderer that serves real forms.
- [Polls](/docs/factory/polls) and [slides](/docs/factory/slides) — asking a
  group a question, and putting the answer on a lecture screen.
- [Notebooks](/docs/factory/notebooks) — publishing something that runs.
- [The front door](/docs/features/frontdoor) — what happens at home to what the
  factory collects.
