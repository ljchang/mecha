---
title: Interfaces
sidebar_position: 1
description: The four front ends over one agent loop, and why only the full-screen one can steer a run in flight.
---

# Interfaces

One agent loop, four ways to drive it. `mecha run` answers a single task and
exits. `mecha chat` is a readline REPL. `mecha tui` is full-screen, and the
input line stays live while the agent works. `mecha batch` fans the same agent
out over a file of prompts.

The loop itself is the same code in all four. What differs is who owns the
terminal, and that single fact decides what each front end can do — most
visibly, whether you can redirect a run without stopping it.

## `mecha run` — one task, one answer

```bash
mecha run "summarize what changed in this repo today"
mecha run --json "list the notes and count them"
mecha run --resume 20260805T091500-3f2a "and what about Thursday?"
echo "explain this" | mecha run -
```

The prompt can be an argument, `-`, or omitted, in which case it is read from
stdin. Output streams by default; `--no-stream` waits for the whole answer,
`--quiet` drops the tool narration, and `--json` emits one object with the
text, stop reason, turn count, usage, cost, and the session id.

Two details worth knowing:

- **Approval depends on whether anything can answer.** `run` treats the run as
  interactive only when stdin is a terminal *and* `--json` was not passed.
  Otherwise it falls back to the configured permission mode, where `ask` means
  no — there is nobody to say yes, and the safe reading of a question nobody
  hears is a refusal.
- **Exit codes carry the outcome.** `0` success, `1` error, `2` the model
  refused, `3` it produced no answer at all. A script can tell the three
  apart without parsing prose. A run stopped by a turn or token ceiling that
  still answered exits `0` — the work it left behind is graded on its own
  terms, and `--json`'s `stop_cause` names the ceiling for callers that care.

`--resume <id>` continues a saved transcript, and the taint recorded in it
comes back with it — see [Sessions and replay](/docs/features/sessions-and-replay).

## `mecha chat` — a REPL

```bash
mecha chat
mecha chat --resume 20260805T091500
```

One `Conversation` for the whole session, which is the point: taint travels
with the messages, so a hostile page read on turn one still arms the interlock
on turn five. `/clear` starts a genuinely new conversation, taint included,
because nothing the old one read is in context any more.

Slash commands are `/tools`, `/model`, `/usage`, `/clear`, `/session`,
`/exit`, and `/help`. Ctrl-C abandons the line you are typing; Ctrl-D ends the
session. History is written per line to `~/.mecha/sessions/chat_history`, so a
killed process keeps what was typed before it died — and it lives beside the
transcripts because the sessions directory is owner-only and a typed prompt
deserves the same protection as the transcript recording it.

A failed turn is rolled back rather than left dangling: if the request errors,
the user message is dropped, so the next request does not resend a turn that
never got a reply.

## `mecha tui` — full-screen, and steerable

```bash
mecha tui
```

A single event loop owns the terminal for the session and the agent runs in a
task beside it. That is the whole reason this exists rather than a third REPL,
and it is what makes steering possible — see below.

Keys, from the `?` overlay:

| Key | What it does |
|---|---|
| `enter` | send — while running, steer the run |
| `alt+enter` (`shift+enter` under the kitty protocol) | insert a newline |
| `tab` | complete a `/command` or an `@path` |
| `shift+tab` | toggle planning, which hides the writing tools |
| `^o` | show or hide thinking and tool output |
| `^c` | stop the run; twice at idle to quit |
| `^d` | quit, when the input is empty |
| `esc` | jump back to the newest output |
| `^g` | compose the input in `$EDITOR` |
| `!command` | run it locally — the model never sees it |

Slash commands go further than `chat`'s, because the TUI is the only front end
that can change anything mid-session: `/model`, `/provider` and `/mode` switch
what is answering, `/mcp` turns servers on and off individually or wholesale,
`/todo` shows the live task list, and four modals open onto the review surfaces:
`/triggers` (see [Triggers](/docs/features/triggers)), `/outbox` (see [the
outbox](/docs/features/outbox)), `/frontdoor` (see [the front
door](/docs/features/frontdoor)) and `/polls` (see
[Polls](/docs/factory/polls#watching-one-without-leaving-the-session)). Each
drives the matching `mecha …` or `factory-publish …` child process rather than
reimplementing it, so nothing the modal can do is missing from the command line.
A typo'd command is reported as unknown rather than sent to the model as a
prompt.

The status line becomes a fuel gauge when `[providers.X] context_window` is
set — `context 29.3k/32.8k (89%)`, grey below 75%, yellow to 89%, red above.
Without a configured window it shows the prompt size with nothing to compare
it to. See [Compaction](/docs/features/compaction).

Testing the TUI means driving a pty, and giving it a size:

```bash
script -qec "stty rows 45 cols 130; mecha tui" /dev/null
```

A pty with no window size renders every frame into a 0x0 area.

## Cancel and steer are different things

This is the distinction the interfaces exist to express.

### Cancel stops the run and keeps what it has

`RunContext::cancel` holds a `CancellationToken`. The loop checks it at the top
of every turn, and mid-turn a `tokio::select!` races the provider future
against the token. Losing that race **drops the provider future**, which is
what aborts the in-flight HTTP request — cancellation in Rust is a dropped
future; there is nothing else to abort.

Because the future is dropped, the accumulated text has to live outside it. It
does: the partial answer and the usage so far are held in `Arc<Mutex<...>>`
alongside the stream, so a cancelled turn keeps what the model had written and
what the prompt cost. That is why **a cancellable run always streams**. Without
a stream there is no partial answer to keep, and `RunContext::cancel` is opt-in
rather than always-on for exactly that reason — a batch worker nobody can
interrupt should not silently switch transports.

Tools are never interrupted mid-call. Cancellation stops the run at the next
safe point: a turn boundary, or the model call itself. The run ends with
`StopCause::Interrupted` and the partial text as its answer.

In `run` and `chat`, Ctrl-C is wired to this by `run_interruptible`. The signal
is watched in a separate task rather than selected against the run, because
selecting would drop the *run* future and throw away the very partial answer
cancellation exists to preserve. The first Ctrl-C cancels; a second is left to
the default handler, so a wedged run is still killable.

```
^C — stopping after the current step. Ctrl-C again to force.
```

In the TUI, Ctrl-C cancels the run and the status line says `stopping`. At idle
it takes two presses to quit.

### Steer redirects a run without stopping it

`RunContext::queued_input` is a queue the caller can push into while a run is in
flight. The loop drains it at the top of each turn and folds the text into the
message that already carries the tool results, so the model reads "here is what
your tools returned, and also: actually, focus on X" as one user turn and keeps
working.

That placement is not a detail. Between an assistant's `tool_use` and its
results there is no valid slot for a user message — the API requires a result
for every call, and two user messages in a row are invalid — so the first legal
opening is the results message itself. Taking it is what makes steering
mid-run possible at all, rather than merely queued until the run ends.

The queue is drained, so a steer is delivered exactly once; leaving it in place
would re-send it on every subsequent turn. Text queued before any tool call
becomes its own user message, which is the only legal shape available there.

The cost is latency: a steer waits for the in-flight model call and the tools it
asked for. Interrupting sooner would mean discarding a turn already paid for.

### Why only the TUI can steer

Steering needs a single owner of stdin.

A readline REPL owns stdin only *between* runs. Reading it while a run streams
would need a second reader on the same file descriptor, and whichever reader is
blocked when the run ends steals the user's next prompt line. `mecha-cli`'s
`interrupt` module says so in a comment where the consumer would otherwise go:
the queue has no consumer there on purpose.

The TUI has one event loop owning the terminal for the whole session, with a
persistent input area, so a line submitted mid-run has somewhere unambiguous to
go. In `submit`, shell escapes (`!git status`) and slash commands are handled
*before* steering — a `/clear` typed mid-run is far more likely to be a mistake
than an instruction for the model, and sending it as steering would put a slash
command into the transcript. Anything else, while a run exists, goes into that
run's queue.

This is a property of the front end, not of the loop. Any caller that owns its
own input can call `RunContext::with_queued_input` and get the same behaviour.

## `mecha batch` — fan-out

```bash
# items.jsonl — one object per line, or a bare JSON string
{"id": "q1", "prompt": "who did I meet with last week?", "meta": {"gold": "..."}}
{"id": "q2", "prompt": ["read the notes", "now summarise them"]}

mecha batch items.jsonl --concurrency 8 --out results.jsonl --yes
```

Bounded concurrency over independent prompts, results keyed by `id` and written
as each finishes — a killed run still leaves everything completed so far on
disk. `--limit` truncates the input for a smoke test over a big file. Duplicate
ids are refused up front, because they make the output impossible to join back.

Decisions that shape it:

- **Each item gets a fresh `Conversation`.** Batch items are independent by
  definition, and sharing history would leak one into the next. That covers
  taint: one item reading a hostile page must not arm the interlock for the
  next, which never saw it.
- **`prompt` may be a list.** Several turns then run on *one* conversation, so
  taint accumulates and the transcript grows exactly as it would in a real
  session. A single string still parses, so no existing file had to change.
  If a turn errors, the item stops there: later turns were written to follow
  it, and running them against a conversation missing a reply measures
  something nobody asked for.
- **Batch runs are unattended.** There is nobody to approve, so `mecha batch`
  warns when neither `--yes` nor `--read-only` was passed, and state-changing
  tools are refused.
- **`run_with` gives each item its own `RunContext`.** That is what makes a
  batch of *mutating* items possible: hand each one a private workspace and
  permission to write to it, and they stop being able to see each other's side
  effects. The eval rig is built on this.

An item is `ok` only when the run was not exhausted, the model did not refuse,
and no tool arguments were malformed. `mecha batch` exits non-zero when anything
failed.

## As a library

All four front ends are thin. `Agent::run` uses the agent's own `RunContext`;
`Agent::run_in` takes a caller's. One agent — one provider connection, one
cached prefix — can serve concurrent runs jailed to different directories under
different permissions.

```rust
let cx = agent.context().as_ref().clone()
    .with_cancel(token.clone())
    .with_queued_input(Arc::clone(&queue));

let outcome = agent.run_in(&cx, &mut convo, Some(events_tx)).await?;
```

See [Providers](/docs/features/providers) for what sits underneath, and
[Tools and MCP](/docs/features/tools-and-mcp) for what the loop dispatches to.
