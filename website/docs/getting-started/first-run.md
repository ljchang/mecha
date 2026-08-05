---
title: First run
sidebar_position: 2
description: Configure a provider, check the tool surface without credentials, then run mecha one-shot, in a REPL, and full-screen.
---

# First run

Four steps: look at the tool surface, configure a provider, run one task, then
try the interactive interfaces.

## 1. `mecha tools` — the smoke test that needs no credentials

`mecha tools` deliberately does not build a provider. You should be able to see
and debug your tool surface before any credentials exist, and a broken MCP
server should be diagnosable without spending a token.

```bash
mecha tools
```

It lists every tool an agent would see right now, prints the active sandbox
backend, and — because it connects to configured MCP servers — doubles as a
check that those servers actually start.

Two flags are worth knowing:

```bash
mecha tools --schema     # the full JSON schema for each tool, exactly as the model sees it
mecha tools --json       # machine-readable, including each tool's declared capabilities
```

The `--json` view is the auditable one. `shell` declaring `external_send: false`
is a claim the sandbox is making on its behalf, and it should be inspectable
without reading source.

On a fresh checkout you should see the built-ins: `fs_read`, `fs_write`,
`fs_edit`, `fs_list`, `shell`, `http_fetch`, and `todo`. `web_search` is absent
until a search backend is configured, because a search tool that always errors
is worse than no search tool.

## 2. Configure a provider

### Anthropic

```bash
export ANTHROPIC_API_KEY=sk-ant-...
mecha config init                 # writes ~/.mecha/config.toml
```

`config init` writes a commented starter file rather than a dump of defaults,
because the point of the file is to show what is adjustable. The provider block
it writes:

```toml
default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
```

`api_key_env` names an environment variable. There is also an `api_key` field
that takes the key inline; prefer the variable, because the inline form puts a
credential in a file on disk.

Model ids are exact strings with no date suffix.

### An OpenAI-compatible server

Anything speaking `/v1/chat/completions` works: llama-server, vLLM, Ollama, or a
hosted API. Add a second provider entry — providers merge by key, so a project
file can add a local endpoint without restating the Anthropic one.

```toml
[providers.local]
kind = "local"                     # "local", "openai", "openai-compatible" — same backend
base_url = "http://127.0.0.1:8080" # no /v1 suffix; the path is appended
model = "qwen3-14b"
context_window = 32768             # the -c the server was started with
```

`context_window` is not optional in practice even though the field is. Nothing
can discover it — a provider reports what a prompt *cost*, never what is left —
and three things degrade silently without it. See
[Configuration](/docs/getting-started/configuration).

Then either make it the default:

```toml
default_provider = "local"
```

or select it per run:

```bash
mecha run -p local "..."
export MECHA_PROVIDER=local        # or via the environment
```

## 3. `mecha run` — one task, one answer

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
| `3` | It ran out of turns (or another budget stopped it) |

Note that `--json` implies non-interactive: nothing can answer an approval
prompt when output is being piped or parsed, so those runs use the configured
permission mode instead of asking.

## 4. `mecha chat` — a REPL

```bash
mecha chat
mecha chat --resume 4f2a      # continue a saved session by id or unique prefix
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

## 5. `mecha tui` — full-screen, and steerable

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
/help  /tools  /triggers  /model  /provider  /mode  /mcp
/usage  /todo  /session  /clear  /quit
```

`/mode ask|allow|read-only` changes the permission mode without restarting.
`/mcp <server> on|off` toggles one server. `/triggers` opens the scheduled-prompt
manager. When `context_window` is configured, the status line becomes a fuel
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
