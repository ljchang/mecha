---
title: Accounts and connecting agents
sidebar_position: 2
description: Claiming a handle, pairing a machine, and wiring the factory into an agent — the door everything else on the public surface is behind.
---

# Accounts and connecting agents

Everything else in this section assumes two things exist: a **handle** (the name
your artifacts are served under) and a **paired machine** (one that holds keys
the box will accept). This page is how you get both, and how an agent on that
machine reaches the factory afterwards.

The whole sequence is a few minutes and happens once per machine.

## 1. Claim a handle

An account starts from an **invite**. Open the signup page the invite points at,
choose a handle, and confirm the address — the handle names you in both of the
URLs an artifact gets, the bytes on their own origin
(`https://<handle>.art.mecha-factory.ai/…`) and the page you send a person
(`https://gate.mecha-factory.ai/view/<handle>/…`; see
[Artifacts](/docs/factory/artifacts#two-urls-and-which-one-to-send)). So it is
worth a moment's thought, and it is not casually changed afterwards.

What you get back is a **pairing code**. It is short-lived and single-use, and it
is the only thing that crosses from the browser to your terminal.

## 2. Pair the machine

```bash
factory-publish connect <code> --gate https://gate.example.org --handle <handle>
```

The code is spent, and the keys it mints are written to `~/.mecha/factory/`. The
handle is typed rather than inferred: the server refuses a mismatch **without
spending the code**, and typing it is the whole confirmation step.

`--label` names these keys in the operator's key list; it defaults to the
machine's hostname, which is what you want when the question later is "which
laptop was that?".

### Pairing is a CLI and deliberately not a tool

An agent on this machine cannot pair it. That is a decision rather than an
omission: pairing decides *what an agent on this machine may do*, and something
an agent could do to itself as a side effect of conversation is not a permission
boundary. The same reasoning keeps `operator`, `connect`, `disconnect`, `serve`
and `drain` off the [MCP surface](#4-wire-it-into-an-agent) — they are the
operator's, not the model's, and the factory has a test that fails the build if
anyone adds one without writing down why.

## 3. What the keys are, and why there are several

Authority is a **scoped key per capability**, not one key that does everything —
and pairing installs exactly two of them:

| Key | File | What it permits | Installed by pairing |
|---|---|---|---|
| `publish` | `publish.key` | Push new bundle versions | **yes** |
| `drain` | `drain.key` | Collect what strangers submitted | **yes** |
| `release` | `release.key` | Move what a share URL resolves to; serve a public form | no |
| `slots` | `slots.key` | Push availability, create and read polls | no |
| `operate` | `operate.key` | The admin panel: accounts, invites, keys | **never** |

**So a paired machine has no release key at all**, and that is the whole point.
Publishing a version puts bytes on the box; releasing decides what the world
sees when it follows a link somebody already has. A machine that renders and
publishes on a schedule does not need to be able to change what an existing link
resolves to — and, more to the point, an agent running on it should not be able
to, however the conversation goes. Separating them puts "a human decides what
goes live" on the *credential*, where nothing said in a prompt can argue with
it, instead of on a habit.

Release authority stays in the browser instead: a signed-in session at the
gate's account page **is** a release credential, driving the same alias move the
key-authenticated endpoint drives. A person moves a link by clicking, from the
machine they review from. `release.key` exists for a machine somebody
deliberately keeps for that job — and pairing a machine that already has one
prints a warning rather than refusing, because you may be pairing exactly that
machine.

`operate` is never installed by pairing at all. It is minted once on the box and
held on whichever machine the human chooses; suspending accounts and minting
invites is not power an agent should acquire as a side effect of conversation,
which is the same rule that keeps the operator commands off the MCP surface.
`slots` is minted on the box the same way and dropped in beside the config when
a machine needs the availability pipeline.

Keys are **files the tools open**, never values in the environment, so a crash
log or an environment dump cannot contain one.

## 4. Wire it into an agent

The factory reaches an agent as an ordinary [MCP server](/docs/features/tools-and-mcp).
Nothing in `mecha-core` knows a public surface exists — that is the founding
invariant on the mecha side, and it is why this is a server rather than a
built-in tool.

```toml
[[mcp]]
name = "factory"
command = "factory-publish"
args = ["mcp"]

# Nothing outbound leaves without a human reading it first.
[outbox]
tools = [
  "factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish",
  "factory__poll_create", "factory__poll_meeting_create", "factory__poll_close",
  "factory__type_push",
]
publish_tools = [
  "factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish",
  "factory__poll_create", "factory__poll_meeting_create", "factory__type_push",
]
```

`mecha tools` lists what the agent can now see, with each tool's capabilities and
whether it is routed through [the outbox](/docs/features/outbox). Use it as the
smoke test — it runs without a provider configured, so it answers "is the server
even starting?" before any model is involved.

### What is routed, and what is not

Every verb that changes what the world can see is staged for review rather than
executed: publishing, aliasing, unpublishing, creating a poll, closing one, and
pushing a request type. Rendering is not — it is local, cheap and reversible, and
making every iteration cost a human review is how a review queue stops being
read.

`poll_close` is routed but is deliberately **not** in `publish_tools`. Its
`resolution` is prose somebody wrote that lands at the top of a public page, so
it is exactly the kind of draft a person should be able to edit before release.
The creation verbs are the other way round: their arguments are ids and file
paths, editing one is not editing the draft, and mining a changed path as a
writing lesson is a mistake mecha has a name for.

### A name that matches no tool is a warning, on every start

If `[outbox] tools` names something the registry does not have — a typo, or a
server that failed to start — mecha says so at startup, because the alternative
is that the real tool executes *unrouted* while the config reads as though it
were under review. That is the silently-degrading-sandbox shape, and it is worth
knowing the warning exists so it is not scrolled past.

## 5. Any MCP client, not just mecha

The surface is plain MCP over stdio, so anything that speaks it can publish here
without knowing mecha exists. That has been exercised: the tool surface was
driven end to end from a different agent over raw stdio — handshake,
`tools/list`, and a render under the `--root` jail. The factory repository's
`docs/SECOND-CLIENT.md` is the onboarding path for that case.

`--root` is worth naming explicitly. Every path a model supplies is resolved and
proved to be inside it before anything touches the filesystem — the same
containment proof mecha applies to its own tools, because an MCP server's
arguments are its own business and mecha's path jail does not reach them. It
defaults to the working directory, which is the run's workspace when mecha
confines the server, and it is printed on stderr at startup so you can see what
it actually is.

## Where to go next

- [Artifacts](/docs/factory/artifacts) — versions, visibility, sharing, takedown
- [Notebooks](/docs/factory/notebooks) — publishing something that runs
- [Polls](/docs/factory/polls) — asking a group a typed question
