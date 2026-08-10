---
title: Artifacts
sidebar_position: 3
description: Versions that never move, an alias that does, who may read a bundle, and what taking one down actually means.
---

# Artifacts

An artifact — a *bundle* — is what an agent made, turned into a URL you can send
someone: a report, a dashboard, a briefing, a notebook. This page is the four
questions people actually ask about one after it exists. How it gets *staged for
review* before it goes anywhere is [Publishing](/docs/features/publishing);
this is about what happens to it afterwards.

## Versions never move; one alias does

```
~/.mecha/bundles/<id>/<version>/     immutable · versioned · never deleted
~/.mecha/work/<producer>/            generated · mutable · disposable · swept
```

Two directories that mean opposite things, and keeping them apart is most of the
design. Publishing copies bytes out of the second into the first, where they get
a **version number and never change again**. A share URL does not point at a
version — it points at an **alias**, and the alias points at a version.

That indirection is the whole feature. Publishing a revision does not invalidate
the link you emailed last week; it moves what that link resolves to. And moving
the alias *back* is how you undo a bad revision, in one command, with the bad
version still on disk as evidence.

```bash
factory-publish list                 # every bundle, versions, where each alias points
factory-publish status <id>          # one bundle's versions and what each was rendered from
factory-publish alias <id> --version 3
```

Two consequences worth knowing before they surprise you:

- **Publishing identical bytes makes no new version.** It returns the existing
  one. A trigger that re-renders an unchanged briefing every morning does not
  accumulate thirty identical versions.
- **Moving an alias is a publication, not bookkeeping.** It changes what every
  link already in the world resolves to, which is why `bundle_alias` is
  outbox-routed exactly like `bundle_publish`.

## Who may read it

Every bundle is `public` or `private`, and **a bundle that has never been either
is private**. The origin serves a private bundle to nobody by default — there is
no window between "published" and "decided who may see it".

```bash
factory-publish publish <id> <dir> --visibility public
factory-publish alias <id> --version 3 --visibility private   # visibility travels with the alias
```

Omitting `--visibility` keeps whatever the bundle already was, so a routine
revision cannot silently make a private thing public.

### Sharing a private bundle with a person

From the viewer's **Manage** menu, grant an address. That address proves itself
the way everything here does — a link mailed to it — and becomes a *viewer
session*, which is deliberately not a tenant session and not an operator one.
Revoke it and the bytes stop.

The mechanism underneath is worth one paragraph, because it explains why revoke
is immediate rather than eventual. The gate decides on *its* origin whether you
may read, then mints a short-lived **capability** and frames the artifact origin
with it. The token in that URL is the entire authority: the artifact origin holds
no session and learns no identity, and the capability **re-proves its grant on
every fetch**. So a revoked share stops the bytes mid-page rather than whenever a
token happens to expire.

There are three oracle rules around it, because a share must not leak what it
protects:

- A visitor with no session gets the same sign-in page whether the bundle is
  private, unshared, or was never published at all.
- The sign-in form answers identically whether or not that address has grants.
- A signed-in reader whose address is not on the list gets the same 404 a
  stranger gets.

"No such thing" and "not for you" are indistinguishable from outside, which is
the only way a private URL is not also a directory of what exists.

## Taking one down

```bash
factory-publish unpublish <id>
```

The share URL stops resolving. **Nothing is deleted** — every version stays on
disk and can be aliased again. Unpublishing is the alias pointing at nothing,
not a removal, and that is deliberate: the common reason to take something down
is that it was wrong or premature, and both of those are states you may need the
bytes to reason about later.

The visibility a bundle had is *kept* rather than flipped to private, because
what a reader gets — "this has been taken down" versus "no such thing" — is a
decision, and it should not change as a side effect of an unrelated command.

If you genuinely want bytes gone, delete the version directory yourself. The
tooling does not offer it, because a one-word command that destroys the only copy
of something a link points at is a bad trade against how rarely anyone needs it.

## What retention will and will not sweep

`mecha work clean` keeps the last `[work] keep` entries per producer (default 10)
and says what it removed. It runs nightly. It only ever touches
`~/.mecha/work/`, never `~/.mecha/bundles/`.

There is one guard worth knowing: **retention never removes anything a published
bundle names as a source.** When you publish, the sources a bundle was rendered
from are recorded on it, and the sweep reads them back. So "regenerate last
week's report" cannot silently lose its input to a cleanup that ran on Tuesday.

The contract there is one field of data rather than a shared type — a mirrored
version directory may carry a `bundle.json` with a `"sources": [...]` array —
and a mirror that does not exist protects nothing, which is correct rather than a
stub.

## Reading one back

```bash
factory-publish fetch <id> --out ./somewhere        # the aliased version
factory-publish fetch <id> --out ./somewhere --version 2
```

This copies out of the **local mirror**, not from the origin — your own bytes,
not third-party content. That distinction is why `bundle_fetch` is marked
read-only and not `openWorldHint` for an agent: nothing it returns crossed a
network, so labelling it as content from outside would be a lie in the
restrictive direction, and would arm the interlock against your own published
output.

## Where to go next

- [Publishing](/docs/features/publishing) — how a publish is staged and reviewed
- [Notebooks](/docs/factory/notebooks) — an artifact that runs in the reader's browser
- [The work directory](/docs/features/work) — where the bytes come from
