---
title: Documents
sidebar_position: 23
description: Google Docs, Sheets and Slides under drive.file — a grant that cannot reach a document nobody handed it.
---

# Documents

mecha can create, read, edit and trash Google Docs, Sheets and Slides. The
interesting part is not the tools; it is what the grant behind them **cannot**
do.

## The scope is the security model

Google offers several ways to reach a document, and they are not priced alike:

| Scope | Tier | What it costs you |
|---|---|---|
| `drive.file` | **non-sensitive** | nothing |
| `documents`, `spreadsheets`, `presentations` | sensitive | a verification review, and no publishing until it passes |
| `drive`, `drive.readonly` | restricted | verification **plus a paid security assessment, re-done annually** |

mecha uses `drive.file`, and not only because it is free. `drive.file` covers
**files this app created, or that you explicitly handed it** — nothing else.
A document nobody gave mecha is not reachable, and no instruction inside a run
can make it reachable.

That is the same shape as the path jail: a boundary you can verify by reading
a scope string, rather than one that has to keep being true in every future
diff. Every other option asks the model's tool surface to be trusted with your
whole Drive and then constrains it with prompting and approvals.

## Two ways a document gets in scope

**Creating it.** Anything mecha creates is in scope permanently, with no
further step. This is the common case and it costs nothing.

```
mecha-docs auth            # consent once
```

**Handing it over.** An existing document is adopted through Google's real
file chooser:

```
mecha-docs pick            # opens the chooser; you choose
mecha-docs list            # what mecha can reach, read back from Drive
```

The choosing happens in Google's UI, outside the model's context window —
nothing a document says can cause more documents to be picked.

Two limits worth knowing before you plan around them. Picking is
**per-document**: choosing a folder puts the folder in scope, not the files
inside it. And there is no way to widen scope from inside a run; `pick` is a
command you run, deliberately.

### Headless machines

Both flows need a browser *somewhere*, and there is deliberately no
device-code option — Google's device flow refuses the client type the file
chooser requires, and two clients would hold two disjoint sets of files, since
a `drive.file` grant belongs to a *(user, client)* pair.

`--paste` covers it instead, and covers it better than a tunnel would:

```
mecha-docs auth --paste
```

It prints a URL, you open it on any machine, and you paste back the
`127.0.0.1` address the browser lands on — which it displays in full even
though nothing is listening. No tunnel, no forwarded port, and no browser on
the machine holding the grant.

## The tools

```toml
[[mcp]]
name = "docs"
command = "~/.cargo/bin/mecha-docs"

# A shared document is other people's words, and a comment is an injection
# vector invisible in the rendered page. Reading must arm the interlock.
[mcp.capabilities]
untrusted_input = true
```

| Tool | |
|---|---|
| `docs_list` | everything mecha can reach |
| `docs_read`, `sheets_read`, `slides_read` | read |
| `docs_create`, `docs_append`, `docs_replace` | write a Doc |
| `sheets_create`, `sheets_write` | write a Sheet |
| `slides_create` | new deck (editing slide content is not yet supported) |
| `docs_trash` | move to the Drive trash |

`docs_replace` is the surgical edit — replace text by quoting it, rather than
by index. When the quoted text is not found it says so and changes nothing,
because a model told "ok" there goes on to describe an edit that never
happened.

## Three capability quadrants

The labeling is the part worth understanding, because one of the three is
easy to get wrong in a way nothing would report.

**Reads** are `readOnlyHint` and deliberately *not* `openWorldHint`. A
document fetch travels only to Google, which already holds the file. But the
contents are other people's words, so the `untrusted_input` override above
makes reading arm the trifecta interlock — exactly as reading mail does.

**Writes** are `openWorldHint`, and this is the leg that is easy to miss:
**writing into a document a third party can read is exfiltration.** It looks
like a local edit and it is a publish, with far more bandwidth than a URL's
query string. So every write is named in `[outbox] tools` and stages for your
review rather than executing.

```toml
[outbox]
tools = [
  "docs__docs_create", "docs__docs_append", "docs__docs_replace",
  "docs__sheets_create", "docs__sheets_write", "docs__slides_create",
]
```

**`docs_trash` is neither**, and must not be added to that list. It moves your
own file to your own trash and reaches nobody, so routing it through the
outbox would make review circular — a queue you clear in order to fill
another. But it is not `readOnlyHint` either, or a read-only unattended run
could empty a folder at seven in the morning. It carries `destructiveHint`
alone and sits with the approver. This is the same quadrant `mail_triage`
occupies.

There is **no permanent-delete verb**, and **no sharing or permissions verb**.
`drive.file` would permit both. Trash is reversible where delete is not, and
changing who can read a document is the one action where a successful
injection costs the whole corpus rather than one file — so the boundary there
is the tool surface, and a test asserts the absence.

## Where the credential lives

```
~/.mecha/docs/<account>/oauth.json
```

Its own root, not beside the mail grant, though it is the same type with the
same refresh lifecycle. `mecha doctor` reads every `oauth.json` under
`~/.mecha/mail/` *as a mail grant* and checks it covers that provider's triage
scope — so a `drive.file` credential living there would be reported as a
broken mail account, a finding that names the wrong subsystem. Share the type,
never the namespace.

## Setting up the Cloud project

One project, one **Desktop-app** OAuth client, `drive.file` as its only scope,
published to production. Because the scope is non-sensitive, publishing needs
no verification and no assessment — and publishing is what removes the
seven-day refresh-token expiry that Testing status imposes.

One trap: the console may show a banner reading *"Your app requires
verification"* on a project with no sensitive or restricted scopes at all.
That is **brand verification** — a separate, lighter track that only governs
whether your app's name and logo appear on the consent screen. It blocks
nothing. Do not answer it by submitting for review; check the Verification
Center's two cards instead, where *Data access status* will say verification
is not required.
