---
title: Publishing
sidebar_position: 10
description: Staging a publish through the outbox — why a rendered bundle is not a staged email, and what review had to learn.
---

# Publishing

An agent can turn what it made into a durable, versioned URL: a report, a
dashboard, a morning briefing, a notebook. The publisher is
[mecha-factory](https://github.com/ljchang/mecha-factory), wired in as an
ordinary [MCP server](/docs/features/tools-and-mcp) — nothing in `mecha-core`
knows it exists.

```toml
[[mcp]]
name = "factory"
command = "factory-publish"
args = ["mcp"]

[outbox]
tools = ["factory__bundle_publish", "factory__bundle_alias"]
publish_tools = ["factory__bundle_publish", "factory__bundle_alias"]
```

The surface is small: `bundle_render`, `bundle_publish`, `bundle_alias`,
`bundle_unpublish`, `bundle_fetch`, `bundle_list`, `bundle_status`. Rendering
is local and harmless; publishing is the one that reaches the outside world,
so it goes through [the outbox](/docs/features/outbox) like a send does.

## Staging is sink-agnostic; reviewing is not

The outbox generalised to a second kind of outbound action **without a line
changing in `outbox.rs`**, which was the design goal. Every one of its
*review* affordances broke, because all three assume the staged thing is prose
somebody wrote.

So an item carries an `OutboxKind`, set at staging from `[outbox]
publish_tools`:

| | `message` | `publish` |
|---|---|---|
| The reviewable object | the arguments | the **rendered page** |
| `mecha outbox show` | prints the arguments | names the local directory and the file to open |
| `mecha outbox edit` | opens `$EDITOR` | **refused** |
| Mined for `writing` rules | yes | **no** |

`show` on a publish leads with the page rather than the arguments — which are
a path and a visibility flag — names the bundle directory and `index.html`,
and warns when the path is already gone because retention swept it.

`edit` is refused with a message naming the real action: edit the source,
re-render, publish again, which stages a new item. Rewriting a directory path
is not editing the draft.

:::warning[The load-bearing one]
The writing miner **excludes publishes**. A `writing` reflection becomes a rule
in every future run's cached prefix, so mining `diff(args_before, args)` of a
changed *path* would teach voice rules from bookkeeping. That is exactly the
`"Blocked by a hook:"` mistake in a new costume — machine state read as a human
correction — and it has a test named on it for the same reason that one does.
:::

## The kind is config's to declare, never the tool's

The loop must not learn what a publish is, and a third-party MCP server cannot
be trusted to say. Anything not named in `publish_tools` is a `message`, which
is the conservative default: it keeps the arguments visible and the item
mineable.

A name in `publish_tools` that is not also in `tools` **warns on every start**,
like a routed name that matches nothing — it means the tool executes unstaged
while the config reads as though it were under review.

Items written before the field existed load as `message`, which is what they
were.

## An item records the jail it was drafted under

A staged call is a *deferred* tool call, and a tool call means nothing apart
from its workspace. The drafting run said `{"bundle": "site"}` inside
`~/.mecha/work/<producer>/`; `mecha outbox send` runs in another process, hours
later, from wherever the reviewer happens to be standing.

So the item records its workspace, and the release rebuilds the tool surface
rooted there. An absolute path would fail loudly in the wrong place; **a
relative one is worse**, because a same-named directory beside the reviewer
publishes the wrong bytes with no error anywhere.

It is also the stricter jail of the two — the agent's, not the human's — which
is the one [the interlock](/docs/features/security) reasoned about when it let
the call through.

A batch release builds one surface per distinct workspace, lazily, so the
ordinary nine-replies-from-one-run case still starts the MCP servers exactly
once. Items staged before the field existed release against the reviewer's
workspace, which is what they always did.

## Published is not generated

```text
~/.mecha/work/<producer>/       generated · mutable · disposable · cleanable
~/.mecha/bundles/<id>/<ver>/    published · immutable · versioned · never deleted
```

A version is never rewritten and never deleted; a new publish is a new version
and `bundle_alias` moves a name onto it. That is what makes a published URL
safe to send to someone.

It also constrains [retention](/docs/features/work#retention-is-a-policy-not-an-intention):
`mecha work clean` never removes anything a published bundle names as a source,
because "regenerate last week's report" must not silently lose its input.

## Where to go next

- [The outbox](/docs/features/outbox) — the staging machinery this rides on.
- [The work directory](/docs/features/work) — where a bundle is rendered from.
- [The front door](/docs/features/frontdoor) — the inbound half of the same boundary.
- [Polls](/docs/factory/polls) — the other thing the same box serves, and the
  one place a typed answer replaces a stranger's prose.
