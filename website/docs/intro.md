---
title: What mecha is
sidebar_position: 1
description: A standalone agent harness — one loop, any model, native and MCP tools — and a tour of what it does.
---

# What mecha is

mecha is an agent harness: the loop that sits between a language model and the
things it can do. You give it a task, it asks the model, the model asks for
tools, the harness runs them, feeds the results back, and repeats until there is
an answer.

Every project that wants an agent ends up writing that loop, and then writing it
again slightly differently for the next project. mecha exists so it can be
written once and reused. The loop, the tool registry, the MCP client, the
transcripts, and the security controls are a library; the command-line program
is a thin layer on top of it.

## Two crates, plus a third for mail

**`mecha-core`** is the library. It knows nothing about any CLI or any
application. It holds the agent loop, provider-agnostic message types, the tool
trait and registry, an MCP stdio client, session transcripts, batch fan-out, the
eval rig, and the layered config. If you are embedding an agent in your own Rust
program, this is the dependency.

**`mecha-cli`** builds the `mecha` binary. It is deliberately thin — front-end
concerns only: rendering, readline, the full-screen interface, approval prompts.
Anything that would be useful without a terminal belongs in core.

**`mecha-mail`** is a separate crate: a library for Gmail, Google Calendar, and
Outlook mail and calendar over Microsoft Graph, plus three small MCP binaries
that expose it. It is not wired into the loop — mecha talks to it the same way
it talks to any other MCP server, which is the point. No code in `mecha-core` or
`mecha-cli` knows that Google or Microsoft exists, and neither does the model:
it names an *account* (`dartmouth`, `personal`), never a provider. See
[Mail and calendar](/docs/features/mail).

The split is enforced by the direction of the dependency. `mecha-core` cannot
reach up into the CLI, so a feature that only works when someone is watching a
terminal cannot quietly become load-bearing for a scheduled run.

## The shape of the loop

```
prompt ──▶ provider ──▶ does the reply ask for tools?
              ▲                │
              │                ├── no  ──▶ answer, stop
              │                │
              │                └── yes ──▶ interlock ──▶ hook ──▶ approver
              │                                                     │
              └────────── results folded into one user turn ◀───────┘
```

Two invariants carry the design, and both are worth stating plainly because
most of the rest follows from them.

**The loop never learns where a tool came from, or which provider is behind
it.** Both are trait objects. A built-in `fs_read` and a tool exposed by a
third-party MCP server are the same type to the loop; Anthropic and an
OpenAI-compatible local server are the same type to the loop. If code in the
agent loop ever matches on a provider name, the abstraction has leaked.

**A run is described by a `RunContext`**, not by global state: the path jail it
is confined to, the approver that answers permission questions, its budgets, and
optionally a cancellation token and a steering queue. That is what lets one
agent — one provider connection, one cached prompt prefix — serve concurrent
runs jailed to different directories under different permissions. An eval case
that mutates files gets a private workspace while the case beside it stays
read-only.

## What it can do

**Interfaces.** `mecha run` answers one task and exits, with distinct exit codes
so a script can tell success from a refusal from running out of turns.
`mecha chat` is a terminal REPL. `mecha tui` is full-screen, and is the only
interface that can *steer* a run: the input line stays live while the agent
works, so text typed mid-run is folded into the next tool-result message rather
than waiting for the run to finish. `mecha batch` fans the same agent out over a
JSONL file of prompts with bounded concurrency. See
[Interfaces](/docs/features/interfaces).

**Providers.** Anthropic over raw HTTP (there is no official Anthropic SDK for
Rust), and anything speaking OpenAI's `/v1/chat/completions` — llama-server,
vLLM, Ollama, or a hosted API. Transient failures are classified and retried;
terminal ones are not, because the same payload fails the same way.
See [Providers](/docs/features/providers).

**Tools.** Built in: `fs_read`, `fs_write`, `fs_edit`, `fs_list`, `shell`,
`http_fetch`, and `todo`. `web_search` appears when a search backend is
configured — an agent holding a search tool that always errors is worse off than
one with no search tool. Everything else comes from MCP servers, namespaced
`<server>__<tool>` so two servers can both expose a `search`. See
[Tools and MCP](/docs/features/tools-and-mcp).

**Security, enforced structurally rather than by prompting.** Every
model-supplied path is canonicalized and proven to sit inside the workspace
before anything touches disk. Tools declare capabilities — private data,
untrusted input, external send, destructive — and the loop refuses any sending
tool once a conversation holds both private data and untrusted content. That
interlock sits *ahead* of the human approver on purpose: a person clicking "yes"
is exactly what a prompt injection is trying to engineer. Taint lives on the
conversation, not the run, so a turn boundary does not launder it. See
[Security](/docs/features/security) and [Sandbox](/docs/features/sandbox).

**Policy attachment points.** [Hooks](/docs/features/hooks) run commands at
`pre_tool`, `post_tool` and `session_end`, so logging, redaction and policy
attach without editing the loop; `pre_tool` fails closed. The
[outbox](/docs/features/outbox) names tools whose calls are *staged as drafts*
rather than executed, which makes "draft-only, never send" a property of the
harness instead of something each email tool has to implement.

**Unattended operation.** [Triggers](/docs/features/triggers) run a prompt on a
cron schedule — a morning briefing, overnight inbox triage that stages replies
for review. It is a small feature because everything a scheduled run needs
already existed: the outbox stages sends, the interlock refuses exfiltration,
the sandbox confines `shell`, budgets bound the spend.

**Memory, in two different senses.** [Learning](/docs/features/learning) mines
transcripts for the moments you stepped in — a steer, a denied call, a
corrective follow-up — and consolidates them into rules that ride in the system
prompt. Rules have to keep earning their place: a validation ledger measures
them, and rules the ledger convicts are staged for retirement.
[Distillation](/docs/features/distillation) is the other sense — what happened,
rather than how to work — summarising closed sessions into episodes staged to a
knowledge graph.

**Keeping a long conversation alive.** Every turn sends the whole history, so a
long enough session stops being able to send anything.
[Compaction](/docs/features/compaction) evicts superseded tool results first,
then summarises the middle of the transcript, and validates the summary against
the transcript it replaces before installing it.

**Measurement.** [Sessions and replay](/docs/features/sessions-and-replay)
record every run as an append-only transcript and can re-drive one against
today's code. [Evaluation](/docs/features/evaluation) grades a model on the
*tool-call trace* first and the text second, because the hard part of running a
model in a loop is not intelligence but tool-call reliability: a model that is
5% smarter but malforms arguments one call in twenty is worse in a loop, since
every bad call costs a recovery turn.

## Where to go next

- [Installation](/docs/getting-started/installation) — build it from source.
- [First run](/docs/getting-started/first-run) — point it at a provider and get
  an answer.
- [Configuration](/docs/getting-started/configuration) — the layered TOML, and
  the four settings that matter early.
- [Security](/docs/features/security) — read this before giving an agent
  anything private.
- [CLI reference](/docs/reference/cli) — every command and flag.
- [Source on GitHub](https://github.com/ljchang/mecha).
