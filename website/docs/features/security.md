---
title: Security model
sidebar_position: 4
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
| `external_send` | Can transmit data outside the user's control. A plain HTTP GET qualifies: the payload fits in the query string. |
| `destructive` | May destroy or overwrite data. |

What the built-ins declare:

| Tool | Declares |
|---|---|
| `fs_read`, `fs_list` | `private_data` |
| `fs_write`, `fs_edit` | `destructive` |
| `http_fetch` | `untrusted_input` + `external_send` |
| `shell` | `private_data` + `destructive`, and `external_send` unless a sandbox has taken the network away |

MCP tools declare theirs from the server's annotations, and config can force
extra flags on a server with `[[mcp]] capabilities`. That override is a
**union, never an assignment** — config can distrust a server further, never
less. Letting config narrow a declaration would disarm the interlock on the
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

The third is a property of the tool about to run. Once both legs are set, any
tool declaring `external_send` is refused before it executes, and the model is
told why in enough detail to pick another approach — summarise for the user,
or start a fresh session that touches only one of the two.

The refusal is counted on the run outcome as `blocked_sends`, which is what
`mecha eval`'s `expect.blocked_sends` grades.

### It sits ahead of the approver on purpose

The dispatch order for one call is:

```
interlock  →  pre_tool hook  →  approver (the human)  →  execute
```

The interlock is first because **a human clicking "yes" is exactly what an
injection is trying to engineer.** A prompt that has already convinced the
model to exfiltrate has a good chance of producing an approval dialog that
looks reasonable. The rule is structural, not a judgement, so it is applied
before anyone is asked. Hooks come next, and they can narrow policy but never
loosen it; the human comes last.

### The whole turn is gated, not each call in isolation

Taint is updated after a turn's calls execute, because provenance cannot be
known before a call returns. That alone would let a model read a secret and
send it **in the same turn** and see a clean slate at both gates. So the loop
first computes what the turn *will* arm, from the declared capabilities of
every call in the batch, and gates against that. This was found by running it:
a mail read and an `http_fetch` batched into one turn went through.

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

It used to be created fresh inside `run`, which meant a chat turn reset it.
The hole that opened: fetch a hostile page on turn one, read a secret and send
on turn two, and the interlock saw a clean slate both times — while the
attacker's text sat in the model's context the whole while, still able to
steer the model. **A turn boundary is not a security boundary.**

Bundling the taint with the messages makes the right thing the default rather
than something every caller has to remember. Keep the history and you keep the
taint. Start a new `Conversation` — a batch item, a subagent, an eval case, a
trigger fire — and you get a clean one, because you built a new object to do
it.

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
or not the call succeeded.

When `mark_untrusted_output` is on, external content is additionally wrapped
in a marker telling the model to treat it as data rather than instructions.
That is defense in depth and weak on its own — the interlock is the control
that does not depend on the model cooperating.

## Why `http_fetch` is read-only and still a sink

`http_fetch` reports `read_only() == true`, so it skips the approval gate and
runs in parallel with other reads. It touches none of your data.

It also declares `external_send`, because **a GET is an exfiltration
channel**: the secret goes in the query string. Read-only is a statement about
your data; `external_send` is a statement about where bytes can go. `mecha`
keeps them separate so a tool can be honest about both.

`web_search` is the same shape for the same reason — results are
attacker-influenceable, and the query itself is a payload that fits in `?q=`.
Mail reads are the instructive contrast: a mail body is other people's words,
so reads are `untrusted_input`, but a search query travels only to the
provider that already custodies the mailbox, so they are not `external_send`.

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
`block_sends_after_private = true` refuses **any** outbound call once private
data is in context. It is off by default because it breaks "read my notes,
then look something up", and because the better answer for most people is
capability separation — put the search in a subagent with no filesystem
access, so the two never meet.

## The known gap: `shell`

`shell` is universal, and taint tracking cannot see inside a command. A
command can `cat` a secret and `curl` it out, and the loop has no way to
classify that from the argv. So `shell` is deliberately **not** treated as an
untrusted *source*: labelling it one would arm the interlock on every command
and teach people to switch the interlock off.

The mitigation is not a label. It is the [sandbox](/docs/features/sandbox):
confine the command, take away the network, and `shell` stops being a way out
— at which point it stops declaring `external_send`, because something is
enforcing that claim.

:::danger
Do not give an unsandboxed `shell` to an agent that processes untrusted input.
That is the one configuration where the interlock cannot help you.
:::

## Where to go next

- [Sandbox](/docs/features/sandbox) — the enforcement behind `shell`'s label.
- [Hooks](/docs/features/hooks) — mechanical policy, ahead of the human.
- [The outbox](/docs/features/outbox) — outbound calls staged for review.
- [Tools and MCP](/docs/features/tools-and-mcp) — what declares what.
- [Configuration reference](/docs/reference/configuration) — every `[security]` key.
