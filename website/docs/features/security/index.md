---
title: Security model
sidebar_position: 1
description: The path jail and the lethal-trifecta interlock — what mecha enforces structurally rather than by prompting.
---

# Security model

Two things in mecha are enforced by construction rather than by asking the
model nicely: **the path jail**, which decides what a tool may touch, and
**the trifecta interlock**, which decides when an outbound call is refused.
Both live in the loop and in the tool trait, so a tool cannot opt out of
either and a prompt cannot argue its way past them.

Everything else on this page — the sandbox, hooks, the outbox — narrows
further. Nothing loosens these two.

## The path jail

Every model-supplied path goes through `ToolCtx::resolve`, which resolves it
against the run's workspace, canonicalizes it, and proves the result is still
inside:

```rust
let path = ctx.resolve(arg_str(&input, "path")?)?;
```

The order matters. `..`, symlinks, and absolute paths all have to be checked
*after* canonicalization, not before, or a symlink inside the workspace
becomes a way out of it. Because a write targets a file that may not exist
yet, `resolve` canonicalizes the nearest existing ancestor and re-appends the
rest, then checks containment on that.

There is exactly one sanctioned exception: the run's own spill directory,
where an oversized tool result is saved in full so the model can read the
rest back. Its contents are that context's own tool output, so nothing new
becomes reachable — and each re-rooted context gets a fresh spill directory,
so two eval cases cannot read each other's output through a shared one.

:::warning
The convention for anyone adding a tool: **never call `fs::*` on a raw path
from tool input.** The jail is a function you have to call. A tool that skips
it is not jailed, and nothing else in the harness will notice.
:::

The workspace comes from `ToolCtx` at call time, not from global config, which
is what lets one agent serve concurrent runs jailed to different directories.

### A jail has to be rooted somewhere harmless

A correct containment check around the wrong directory contains nothing worth
containing. `~/.mecha/` holds the mail OAuth tokens, every session transcript,
and the learning store — and `$HOME` **contains** `~/.mecha/`, so a jail
rooted at your home directory would cover all of it. Two rules follow:

- `setup` **refuses any workspace that contains the mecha home**. Note the
  direction: a workspace *inside* `~/.mecha/` is fine and is the default for
  unattended runs. What is refused is one the mecha home sits under.
- An unattended run — a [trigger](/docs/features/automation/triggers) with no
  explicit workspace, for instance — defaults to
  [`~/.mecha/work/<producer>/`](/docs/features/automation/work), a directory
  that holds nothing sensitive and that the run is meant to write to, rather
  than to whatever directory the service happened to start in.

```
workspace /home/you contains the mecha home (/home/you/.mecha), so the path
jail would cover the mail tokens, every session transcript and the learning
store.
Run from a project directory instead, or name one explicitly with
`--workspace <dir>`.
```

A path that cannot be canonicalized is compared as written: over-refusing a
workspace is recoverable, under-refusing one is the bug.

The capability interlock below is a backstop, but a backstop is not a
boundary: a jail rooted where the secrets live is a sandbox that has silently
stopped protecting anything.

## Capabilities

Every tool declares what it can do:

```rust
fn capabilities(&self) -> Capabilities {
    Capabilities::default().untrusted().sends()   // http_fetch
}
```

| Flag | Meaning |
|---|---|
| `private_data` | Returns data the user considers private. |
| `untrusted_input` | Returns content a third party can influence — a page, an email body, a calendar invite title. |
| `egress` | Whether data can leave, **and who picks the destination**: `none`, `blind`, or `chosen`. See below — a plain HTTP GET qualifies as egress, because the payload fits in the query string. |
| `destructive` | May destroy or overwrite data. |

The `egress` classes, ordered `none < blind < chosen`:

| Class | Meaning | Examples |
|---|---|---|
| `none` | Nothing leaves. | `fs_read`, a confined `shell` |
| `blind` | The payload is model-authored, but the **recipient is fixed by your config** — or by something the run already received — and appears nowhere in the tool's input schema. An injection can fill the channel and still has nobody to read it back. | `web_search` — its schema is `query`, `limit`, `depth`, with no destination field; `web_open` — its only argument is the handle of a search result |
| `chosen` | **The model names the recipient.** Payload and read-back in one call. | `http_fetch` (`url`), `mail_send` (`to`), a Slack post, an unconfined `shell` |

Blind is earned in code, by a tool whose schema has no destination — there is
deliberately **no configuration that grants it**, because an operator vouching
for a third-party server's destination would be a narrowing nothing enforces.

What the built-ins declare:

| Tool | Declares |
|---|---|
| `fs_read`, `fs_list` | `private_data` |
| `fs_write`, `fs_edit` | `destructive` |
| `http_fetch` | `untrusted_input` + `chosen` egress |
| `web_search` | `untrusted_input` + `blind` egress (`chosen` if no blind backend is configured) |
| `web_open` | `untrusted_input` + `blind` egress |
| `shell` | `private_data` + `destructive`, and `chosen` egress unless a sandbox has taken the network away |

MCP tools get theirs partly by assumption and partly from the server's
annotations. Every MCP tool is assumed to return `private_data` — that is what
most servers exist to do. `openWorldHint` makes a tool `untrusted_input` and
`chosen` egress, and `destructiveHint` makes it `destructive`. No annotation
can say "this returns other people's words", so a server like
[mail](/docs/features/tools/mail#capability-labeling-reads-are-untrusted-sources-not-send-sinks),
whose reads are not open-world, counts as an untrusted source **only because
its `[mcp.capabilities]` block sets `untrusted_input = true`**. Drop that line
and reading an attacker's email arms nothing.

That override is a **union, never an assignment** — config can distrust a
server further, never less. Letting config narrow a declaration would disarm the interlock on the
strength of a claim nothing enforces, and would make the cheapest
configuration the most dangerous one.

## The lethal trifecta, and the interlock

An agent that simultaneously holds **private data**, **untrusted content**,
and **a way to send data out** can be turned into an exfiltration tool by
instructions hidden in the content it reads. No amount of prompting reliably
prevents this: the injected text arrives through the same channel as the
legitimate data.

Two of those three are properties of the transcript, so the loop tracks them:

```rust
pub struct Taint {
    pub private: bool,
    pub untrusted: bool,
}
```

The third is a property of the tool about to run — and specifically of its
egress **class**, because the attack needs the attacker to pick the recipient.
Once both legs are set, any tool whose egress is **`chosen`** is refused before
it executes, and the model is told why in enough detail to pick another
approach — summarise for the user, start a fresh session that touches only one
of the two, or use a route whose destination it does not choose.

A **`blind`** tool is not refused here. `web_search` keeps working in a
conversation holding your mail, because the query reaches the `[[search]]`
backends you configured and nowhere else; an armed conversation is served by
the blind ones only, at quick depth, and the result says so. If you want no
private data reaching a third party at all — attack or not, your own request
or not — that is a different control, `block_sends_after_private`, and it
refuses `blind` and `chosen` alike.

**Opening a result works too.** `web_search` prints a handle in brackets
beside each result, and `web_open` takes that handle — not a URL — and reads
the page. The model chooses *among* the results a search returned and never
writes an address, so there is no field a secret could ride in; what it
leaks is which result it picked. A URL from anywhere else — a link in an
email, one the model made up — still goes through `http_fetch`, which is
refused once both legs are set.

The refusal is counted on the run outcome as `blocked_sends`, which is what
`mecha eval`'s `expect.blocked_sends` grades.

### It sits ahead of the approver on purpose

The dispatch order for one call is:

```
interlock  →  pre_tool hook  →  outbox staging (routed calls stop here)
           →  approval rules ([[rule]])  →  approver (the human)  →  execute
```

The interlock is first because **a human clicking "yes" is exactly what an
injection is trying to engineer.** A prompt that has already convinced the
model to exfiltrate has a good chance of producing an approval dialog that
looks reasonable. The rule is structural, not a judgement, so it is applied
before anyone is asked.

- [Hooks](/docs/features/security/hooks) come next, and they can narrow
  policy but never loosen it.
- A call named in `[outbox] tools` is then **staged, not executed**: it
  becomes a draft for you to release with `mecha outbox`, and never reaches
  the rules, the approver or execution. Staging sends nothing, which is why a
  routed call skips the interlock — the draft records the conversation's
  taint so the review can say so. See [the outbox](/docs/features/security/outbox).
- [Approval rules](/docs/reference/configuration#rule-and-approval) narrow
  what the approver would pass: `forbid` refuses with nobody asked, `prompt`
  asks even for a read-only tool, and `allow` can never soften an escalation
  the interlock asked for.
- The human comes last.

### The whole turn is gated, not each call in isolation

Taint is updated after a turn's calls execute, because provenance cannot be
known before a call returns. That alone would let a model read a secret and
send it **in the same turn** and see a clean slate at both gates. So the loop
first computes what the turn *will* arm, from the declared capabilities of
every call in the batch, and gates against that — so a mail read and an
`http_fetch` requested together are judged as the armed turn they are.

### Policy

```toml
[security]
trifecta = "block"                  # block | ask | allow
mark_untrusted_output = true
block_sends_after_private = false
block_private_ips = true
```

`ask` escalates to a human instead of refusing, which is only meaningful when
someone is watching. `allow` waives the injection interlock and is appropriate
only when the "untrusted" source is in fact trusted.

## Taint belongs to the conversation, not the run

`Taint` lives on `agent::Conversation`, beside the messages:

```rust
pub struct Conversation {
    pub messages: Vec<Message>,
    pub taint: Taint,
}
```

**A turn boundary is not a security boundary.** A hostile page fetched on
turn one is still in the model's context on turn two, still able to steer it,
so a secret read and sent on turn two must be judged against it.

Bundling the taint with the messages makes the right thing the default rather
than something every caller has to remember. Keep the history and you keep the
taint. Start a new `Conversation` — a batch item, an eval case, a trigger
fire — and you get a clean one, because you built a new object to do it.

A **subagent is not a new start**: it gets a fresh conversation but begins
with its parent's taint, because the one message it receives was written out
of everything the parent had read. Delegation does not create a clean
boundary. See [Subagents](/docs/features/tools#subagents).

Two consequences worth knowing:

- **Resuming does not launder it.** Sessions record a `Taint` checkpoint after
  each run, and resuming rebuilds the conversation with
  `Conversation::resumed(messages, taint)`. The plain `From<Vec<Message>>`
  conversion treats messages as clean, which is right for a conversation being
  started and wrong for one being resumed — resuming that way would reopen the
  same hole.
- **Compaction does not launder it either.** Summarising away the text of a
  hostile page does not un-read it. The compaction code never touches
  `Conversation::taint`; the type does the work.

## `untrusted_input` versus `external`

These are two different questions and confusing them causes a real bug.

- `Capabilities::untrusted_input` says what a tool **can** return.
- `ToolOutput::external` says whether **this particular result** actually came
  from outside the machine.

The untrusted leg of the taint, and the wrapper that marks third-party content
to the model, both key off `external`:

```rust
taint.untrusted |= caps.untrusted_input && out.external;
```

Without that, a refusal generated by mecha's own guard — an SSRF block, a
domain-policy denial — would be labelled third-party content, and the model
would start inventing explanations for its own harness's behaviour.

The rule for tool authors: **any tool that reaches the network must call
`.from_outside()` on its output.** A body is third-party content even on a
4xx; an injection hides just as well in an error page.

The private leg is different and deliberately so: it is set from the declared
capability, because a tool that reads your files has read your files whether
or not the call succeeded. An [image you attach](/docs/features/interfaces/images)
arms it too — a screenshot is captured, not composed, so it can hold anything
that was on the screen.

When `mark_untrusted_output` is on, external content is additionally wrapped
in a marker telling the model to treat it as data rather than instructions.
That is defense in depth and weak on its own — the interlock is the control
that does not depend on the model cooperating.

## Why `http_fetch` is read-only and still a sink

`http_fetch` reports `read_only() == true`, so it skips the approval gate and
runs in parallel with other reads. It touches none of your data.

It also declares **`chosen`** egress, because **a GET is an exfiltration
channel** and the model picks the host: the secret goes in the query string
and the attacker's server reads it out of an access log. Read-only is a
statement about your data; egress is a statement about where bytes can go and
who decides. `mecha` keeps them separate so a tool can be honest about both.

`web_search` is the instructive near-miss. Its results are
attacker-influenceable and its query is a payload that fits in `?q=`, so it
is genuinely egress — but its input schema has **no destination field**, so
the payload goes to the `[[search]]` backends you configured and an injection
cannot redirect it. That is `blind` egress: the trifecta interlock leaves it
alone and `block_sends_after_private` still refuses it.

Mail reads are the other contrast: a mail body is other people's words, so
the mail server is configured with `untrusted_input = true` (see
[Capabilities](#capabilities) — the override, not an annotation, is what makes
it so), but a search query travels only to the provider that already
custodies the mailbox, so reads are not egress at all.

Alongside the capability model, `http_fetch` refuses loopback, private,
link-local (including the `169.254.169.254` metadata endpoint) and CGNAT
addresses when `block_private_ips` is on, pins the connection to the addresses
that passed that check (a re-resolve is the classic rebinding TOCTOU), and
does **not** follow redirects — a public host can otherwise 302 straight to an
internal one. The model is told the redirect target and may re-request it.

## A second control, for a different threat

The interlock stops an *injection* driving exfiltration. It deliberately
allows a send that happens before any third-party content exists, because
nothing could have influenced it yet.

That leaves an ordinary privacy leak: the agent putting your private data into
an outbound call because you asked it to, or because it judged that helpful.
`block_sends_after_private = true` refuses **any** outbound call — `blind` or
`chosen` — once private data is in context. It is off by default because it
breaks "read my notes, then look something up".

Moving the lookup into a [subagent](/docs/features/tools#subagents) does not
get around it. A child starts with its parent's taint, and a subagent's egress
is the highest among its own tools, so once private data is in the parent's
context, delegating to any child that can send is refused by this guard too. A
child whose only sender is `blind` — a search-only research profile — is
still let through by the *interlock* in an armed conversation, but never by
this guard. If you turn it on, do the lookup before the private data arrives,
or in a separate session.

## The known gap: `shell`

`shell` is universal, and taint tracking cannot see inside a command. A
command can `cat` a secret and `curl` it out, and the loop has no way to
classify that from the argv. So `shell` is deliberately **not** treated as an
untrusted *source*: labelling it one would arm the interlock on every command
and teach people to switch the interlock off.

The mitigation is not a label. It is the [sandbox](/docs/features/security/sandbox):
confine the command, take away the network, and `shell` stops being a way out
— at which point its egress drops to `none`, because something is enforcing
that claim.

:::danger
Do not give an unsandboxed `shell` to an agent that processes untrusted input.
That is the one configuration where the interlock cannot help you.
:::

## Where to go next

- [Sandbox](/docs/features/security/sandbox) — the enforcement behind `shell`'s label.
- [Hooks](/docs/features/security/hooks) — mechanical policy, ahead of the human.
- [The outbox](/docs/features/security/outbox) — outbound calls staged for review.
- [Tools and MCP](/docs/features/tools) — what declares what.
- [Configuration reference](/docs/reference/configuration) — every `[security]` key.
