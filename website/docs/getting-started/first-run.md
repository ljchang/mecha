---
title: First run
sidebar_position: 3
description: Start mecha three ways — one-shot, as a REPL, and full-screen — and know where it writes things and what to do when a run goes wrong.
---

# First run

Three ways to start it, in increasing order of how much of a conversation you
want. They share everything that matters — the same agent, the same tools, the
same session records — and differ in who is holding the keyboard.

If `mecha tools` does not yet list what you expect, or a provider is not
configured, go back to [Setting up](/docs/getting-started/setting-up) first.

## 1. `mecha run` — one task, one answer

```bash
mecha run "summarize what changed in this repo today"
```

The working directory is the workspace: the agent may read anything inside it,
and every model-supplied path is canonicalized and proven to sit inside it
before anything touches disk. `..`, symlinks out, and absolute paths elsewhere
are refused. `-w /some/dir` points it somewhere else.

By default the agent reads freely and **asks before it writes or runs a
command**. Two flags change that:

```bash
mecha run --yes "fix the failing test in src/parse.rs"     # approve everything
mecha run --read-only "explain how the retry logic works"  # refuse anything that isn't a read
```

`--yes` is what unattended runs need. `--read-only` is the right default for
anything pointed at a repository you have not read.

Useful additions:

```bash
mecha run -v "..."                    # narrate tool calls, results and token usage
mecha run --json "..."                # one JSON object instead of prose
mecha run --resume <session-id> "..." # continue a saved conversation
mecha run --max-cost 0.50 "..."       # stop once the run has cost this much
echo "long prompt" | mecha run -      # read the prompt from stdin
```

Exit codes are distinct so a script can tell the cases apart:

| Code | Meaning |
|---|---|
| `0` | Completed |
| `1` | Error |
| `2` | The model refused |
| `3` | It produced no answer at all |

Exhaustion is deliberately not a failure code. A run stopped by a turn, token or
cost ceiling that still answered exits `0` — the work it left behind is graded
on its own terms, and `--json`'s `stop_cause` names the ceiling for callers that
care which one it was.

Note that `--json` implies non-interactive: nothing can answer an approval
prompt when output is being piped or parsed, so those runs use the configured
permission mode instead of asking.

## 2. `mecha chat` — a REPL

```bash
mecha chat
mecha chat --resume 20260805T091500   # continue a saved session by id or unique prefix
```

Readline history, and slash commands:

```
/tools          list available tools
/model          show the active model and provider
/usage          tokens used this session
/clear          forget the conversation so far
/session        show the transcript path
/exit           quit
```

One conversation runs for the whole session, and that is a security property
rather than a convenience: taint travels with the conversation, so a hostile
page read on turn one still arms the interlock on turn five. `/clear` starts a
new conversation, taint included — nothing the old one read is in context any
more, so nothing it read should still apply.

## 3. `mecha tui` — full-screen, and steerable

```bash
mecha tui
```

Same shape as `chat`, so switching between them is muscle memory — `--resume`
and `--no-session` work identically. The difference is that the input line stays
live while the agent is working.

That is what makes **steering** possible. Text typed mid-run does not stop the
run and does not wait for it: it is folded into the message that carries the
tool results, so the model sees the results and the new instruction as one user
turn and keeps going. Cancelling (Ctrl-C) is the other thing, and is
deliberately different — it stops the run at the next safe point and keeps the
partial answer.

Only the TUI can steer, and that is a property of the front-end rather than of
the loop: steering needs a single owner of stdin, which a readline REPL cannot
be while a run is streaming.

The TUI has a longer command list than `chat`, because it is the only interface
that can change things mid-session:

```
/help  /tools  /triggers  /outbox  /frontdoor  /polls  /review
/model  /provider  /mode  /mcp  /usage  /clear  /session  /todo
/exit
```

`/mode ask|allow|read-only` changes the permission mode without restarting.
`/mcp <server> on|off` toggles one server. `/triggers`, `/outbox` and
`/frontdoor` open the scheduled-prompt, staged-send and inbound-request
managers. `/review now|later|auto` decides what happens to drafts a run stages —
set only by slash command, never inferred from the prompt, because release
policy must not be decidable by anything sharing a context window with
third-party text. When `context_window` is configured, the status line becomes a fuel
gauge — `context 29.3k/32.8k (89%)` — instead of a token count with nothing to
compare it against.

## Where things are written

Every run writes an append-only JSONL transcript to `~/.mecha/sessions`
(`MECHA_SESSION_DIR` overrides it, `--no-session` opts out).

```bash
mecha sessions list
mecha sessions show <id>
mecha sessions path <id>
```

The transcript is the record, and several other features read it back rather
than keeping a second copy that could disagree with it. See
[Sessions and replay](/docs/features/sessions-and-replay).

## When something goes wrong

```bash
MECHA_LOG=debug mecha run "..."     # internal tracing, on stderr
mecha config show                   # the merged configuration actually in effect
mecha config path                   # which files are being read, and whether they exist
```

`mecha config show` is usually the fastest answer to "why is it using that
model": it prints the result of every layer merged together, not the contents of
any one file.

## Next

- [Configuration](/docs/getting-started/configuration) — the layered TOML and
  the settings that matter early.
- [Interfaces](/docs/features/interfaces) — run, chat, tui and batch in depth.
- [Security](/docs/features/security) — what the harness refuses to do, and why.
