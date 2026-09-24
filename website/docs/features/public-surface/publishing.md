---
title: Publishing
sidebar_position: 3
description: Staging a publish through the outbox — why a rendered bundle is not a staged email, and what review had to learn.
---

# Publishing

An agent can turn what it made into a durable, versioned URL: a report, a
dashboard, a morning briefing, a notebook. The publisher is
[mecha-factory](https://github.com/ljchang/mecha-factory), wired in as an
ordinary [MCP server](/docs/features/tools) — nothing in `mecha-core`
knows it exists.

```toml
[[mcp]]
name = "factory"
command = "factory-publish"
args = ["mcp"]
[mcp.capabilities]
untrusted_input = true   # poll answers and box reads are other people's text

[outbox]
tools = [
  "factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish",
  "factory__poll_create", "factory__poll_meeting_create", "factory__poll_close",
  "factory__type_push", "factory__surface_push", "factory__surface_pull",
]
publish_tools = [
  "factory__bundle_publish", "factory__bundle_alias", "factory__bundle_unpublish",
  "factory__type_push",
]
```

The two poll creates are routed but **not** publications, on purpose: what
the owner approves there is an invitation in their own name, so the card is
reviewed as a message — readable as prose, editable, and its unedited release
counted as the writing signal it is. A `publish`-kind card would lead with
local paths that do not exist and refuse `edit` on the owner's own sentence.

Eighteen tools, in five families:

| Family | Tools | Reaches the box |
|---|---|---|
| Bundles | `bundle_render`, `bundle_publish`, `bundle_alias`, `bundle_unpublish`, `bundle_fetch`, `bundle_list`, `bundle_status` | `publish`, `alias`, `unpublish` |
| Polls | `poll_create`, `poll_meeting_create`, `poll_status`, `poll_close` | all four |
| Notebooks | `notebook_render` | no |
| Request types | `type_check`, `type_push`, `type_list` | `push`, `list` |
| Surfaces | `surface_push`, `surface_pull`, `surface_list` | all three |

The surfaces are your public pages' own records — a profile, the hangar, or
one switchboard, each a local TOML file. `surface_push` sends one to the box
and changes what a stranger sees at your public pages, so it is a publication
like `type_push`; `surface_pull` writes the box's copy back over the local
file (the cockpit can edit these too, and a push from a stale file overwrites
what was changed there); `surface_list` names every board the box holds and
which were edited in the cockpit since their last push.

The division that matters is **local versus outbound**. `bundle_render` and
`type_check` do their work on your machine and touch nothing, and
`notebook_render` touches nothing either unless asked to vendor the runtime —
`vendor_runtime` fetches Pyodide from a pinned allowlist; `bundle_fetch`,
`bundle_list` and `bundle_status` read your own records. Everything in the
right-hand column carries `openWorldHint` except the three reads —
`poll_status`, `type_list` and `surface_list`, which are declared read-only:
nothing a model writes leaves through them. `openWorldHint` in mecha sets
**both** `untrusted_input` and `chosen` egress (a remote schema is the
server's to write, so nothing local can prove it holds no destination) — so
those are [trifecta](/docs/features/security) sinks, and the ones that change
what the world can see go through [the outbox](/docs/features/security/outbox)
exactly as a send does. The reads are not sinks, but what they return is
still third-party text — a poll's free-text answers are other people's words —
and nothing marks it so except the `[mcp.capabilities] untrusted_input = true`
line on the server block above. See [the onboarding guide](/docs/features/public-surface/onboarding) for the routing to
copy.

`type_push` is the one to look at twice: uploading a request-type manifest is
what makes a public form **exist and start accepting submissions from
strangers**. It is a publication, not bookkeeping, which is why it is staged
like one — and why the other end of it is [the front
door](/docs/features/public-surface/frontdoor).

## Staging is sink-agnostic; reviewing is not

A publish is staged exactly like a message, but it cannot be reviewed like
one: every message affordance assumes the staged thing is prose somebody
wrote. So each item carries a kind, set at staging from `[outbox]
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
changed *path* would teach voice rules from bookkeeping — machine state read
as a human correction.
:::

## The kind is config's to declare, never the tool's

The loop must not learn what a publish is, and a third-party MCP server cannot
be trusted to say. Anything not named in `publish_tools` is a `message`, which
is the conservative default: it keeps the arguments visible and the item
mineable.

A name in `publish_tools` that is not also in `tools` **warns on every start**,
like a routed name that matches nothing — it means the tool executes unstaged
while the config reads as though it were under review.

## An item records the jail it was drafted under

The drafting run said `{"bundle": "site"}` inside its own work directory;
`mecha outbox approve` runs later, from wherever you happen to be standing. The
item records the workspace the tool would have executed in, and both `show` and
the release resolve paths there — so a relative bundle path names the same
bytes you reviewed, never a same-named directory beside you. The details, and
the one case where you should hand a server an absolute path, are on
[the outbox page](/docs/features/security/outbox#an-item-records-the-jail-it-was-drafted-under).

## Published is not generated

```text
~/.mecha/work/<producer>/       generated · mutable · disposable · cleanable
~/.mecha/bundles/<id>/<ver>/    published · immutable · versioned · never deleted
```

A version is never rewritten and never deleted; a new publish is a new version
and `bundle_alias` moves a name onto it. That is what makes a published URL
safe to send to someone.

It also constrains [retention](/docs/features/automation/work#retention-is-a-policy-not-an-intention):
`mecha work clean` never removes anything a published bundle names as a source,
because "regenerate last week's report" must not silently lose its input.

## Where to go next

- [The outbox](/docs/features/security/outbox) — the staging machinery this rides on.
- [The work directory](/docs/features/automation/work) — where a bundle is rendered from.
- [The front door](/docs/features/public-surface/frontdoor) — the inbound half of the same boundary.
- [Polls](/docs/features/public-surface/polls) — the other thing the same box serves, and the
  one place a typed answer replaces a stranger's prose.
