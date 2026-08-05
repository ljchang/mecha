---
title: Configuration
sidebar_position: 3
description: The layered TOML config — where the files live, how layers merge, and the four settings worth getting right early.
---

# Configuration

Configuration is layered TOML. Each layer overrides only the fields it names, so
a project file can change one setting without restating everything above it.

## The layers

In order, later winning:

1. Built-in defaults.
2. `~/.mecha/config.toml` — the global file.
3. `./mecha.toml` — project-local, read from the working directory.
4. `MECHA_PROVIDER`, `MECHA_MODEL`, `MECHA_EFFORT`.
5. CLI flags.

```bash
mecha config init              # write a starter ~/.mecha/config.toml
mecha config init --project    # write a starter ./mecha.toml instead
mecha config path              # which files are being read, and whether they exist
mecha config show              # the merged result — what is actually in effect
```

`mecha config show` is the one that answers questions. It prints the merged
configuration rather than the contents of any single file, which is usually what
you wanted to know.

### What merges, and what replaces

Scalars merge field by field. Tables of things do not:

- **Providers merge by key.** A project file can add `[providers.local]` without
  redeclaring the Anthropic entry.
- **`[[mcp]]`, `[[hook]]`, `[[subagent]]` and `[[search]]` replace wholesale.**
  Merging lists by name would make it impossible for a project to turn a global
  server or hook *off*, and a project that cannot disable an inherited hook
  cannot be trusted to run anything.

### One thing that never comes from a project file

Triggers — scheduled unattended prompts — are not declarable in config at all.
They live as individual files in `~/.mecha/triggers/`, and a trigger run loads
the global file only, with no project layer.

The reason is that `mecha.toml` arrives with a cloned repository, and it can
name MCP servers to spawn, hooks to execute, and tools to enable. That is a
reasonable bargain for someone who has just decided to work in that repository
and is sitting there watching. It is no bargain at all for a run firing at 03:00
with nobody present. See [Triggers](/docs/features/triggers).

## The settings that matter early

Four settings are worth thinking about before anything else. Everything else has
a defensible default.

### Provider and model

```toml
default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
```

`kind` selects the backend: `anthropic` speaks the Anthropic API, and `openai`,
`openai-compatible` and `local` are three names for the same
`/v1/chat/completions` client. `api_key_env` names an environment variable
holding the key — preferred over the inline `api_key`, which puts a credential
in a file on disk.

Per-run overrides:

```bash
mecha run -p local -m qwen3-14b "..."
MECHA_PROVIDER=local MECHA_MODEL=qwen3-14b mecha chat
```

If you want cost budgets or dollar figures in the run summary, prices are
required and both halves must be given:

```toml
[providers.anthropic]
input_price_per_mtok = 5.0
output_price_per_mtok = 25.0
```

Knowing one price is worse than knowing neither, because it silently
under-reports. Leave both unset for a local model and cost is reported as null
rather than a misleading zero.

### `[agent] timezone`

```toml
[agent]
timezone = "America/New_York"
```

Set this. The machine may well run in UTC, and the model has no clock at all, so
without it every "what's on Thursday" is answered several hours off — and wrong
in the worst way, because the times stay internally consistent with each other
and read as correct.

It rides in the system prompt with today's date. The mail MCP servers read the
same zone from `MECHA_TZ`, which you set in their `[[mcp]]` `env` block, so they
render event times in it before the model ever sees them.

An IANA name (`America/New_York`), not an offset, because an offset is wrong
twice a year. An unrecognised name warns and falls back to the machine's zone
rather than failing the run.

### `context_window`

```toml
[providers.local]
context_window = 32768        # the -c llama-server was started with
```

This one is on the *provider*, not on `[agent]`, because it is a property of the
model as served. Nothing can discover it: a provider reports how many tokens a
prompt used, never how many are left.

Three things depend on it, and without it all three degrade silently:

- **The compaction threshold derives from it** — two thirds of the window,
  unless `compact_at_tokens` says otherwise. That turns compaction from
  something you must remember to configure into something that works.
- **The TUI status line becomes a fuel gauge** — `context 29.3k/32.8k (89%)`,
  yellow at 75%, red at 90% — instead of a number with nothing to compare it to.
- **Overflow recovery** knows what it is recovering from. A prompt that does not
  fit is refused outright, and the loop compacts and retries the same turn once.

If you change the server's `-c`, change this to match. A stale value is worse
than none, because the derived threshold trusts it.

### `[agent] compact_at_tokens`

```toml
[agent]
compact_at_tokens = 20000     # or set context_window and let it derive
```

Every turn sends the whole history, so a long enough session stops being able to
send anything. Once the *reported* prompt size passes this threshold, the middle
of the transcript is summarised. Reported rather than estimated, so it counts
cached tokens too.

It is unset by default, and that is deliberate: compaction is lossy, and
paraphrasing someone's conversation because it got long is their decision to
make. Setting `context_window` gets you the derived threshold, which is the
better route — the fraction leaves a third of the window free because the check
happens *between* turns, and the next request still has to fit a reply plus
whatever a burst of parallel tool results adds.

`--compact-at N` sets it for one run. See
[Compaction](/docs/features/compaction) for what actually happens when it fires,
including the eviction pass that runs first and the validation pass that checks
the summary before installing it.

## A worked starting config

```toml
default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
# context_window = ...        # set it to this model's window, in tokens
input_price_per_mtok = 5.0
output_price_per_mtok = 25.0

[providers.local]
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "qwen3-14b"
context_window = 32768

[agent]
timezone = "America/New_York"
max_turns = 40
max_tokens = 64000
effort = "high"              # low | medium | high | xhigh | max
thinking = true
cache_prompt = true

[tools]
permission_mode = "ask"      # ask | allow | read-only
shell_timeout_secs = 120

[security]
trifecta = "block"           # block | ask | allow
```

`permission_mode` is the default answer when nothing is watching to approve a
call; `--yes` and `--read-only` override it per run. `trifecta = "block"` is the
interlock: once a conversation holds both private data and untrusted content,
outbound tools are refused. Read [Security](/docs/features/security) before
loosening it.

## Adding a project layer

A `mecha.toml` beside your code is the right place for things that are true of
that project and nothing else:

```toml
# ./mecha.toml — this repository only
[agent]
system_prompt_file = "prompts/agent.md"
max_turns = 60

[tools]
disabled = ["http_fetch"]
```

Remember that it is read from the working directory, and that a `mecha.toml` you
did not write is code you did not read. Nothing in it can reach a scheduled
trigger run, but it does shape every interactive run started in that directory.

## Adding a field to `Config`

If you are contributing: a new field on `Config` is **two edits, not one**.
Files are parsed into a `ConfigLayer` where every field is optional (that is
what lets a project file override one setting), and a field added to `Config`
alone makes its whole TOML table a *parse error* that kills startup — while
every unit test stays green, because tests build the types directly.

That is exactly how hooks shipped unreachable. A round-trip test
(`every_field_of_config_is_reachable_from_a_file`) serialises a default config
and parses it back through the layer, so the mismatch now fails in CI rather
than in someone's config file.

## The exhaustive list

This page covers four settings out of several dozen. Sandbox backends, search
backends, MCP servers, subagent profiles, hooks, the outbox route, retry and
fallback policy, and every security flag are documented in the
[configuration reference](/docs/reference/configuration).

## Next

- [Providers](/docs/features/providers) — retries, fallbacks, and what each
  backend accepts.
- [Tools and MCP](/docs/features/tools-and-mcp) — adding tools from MCP servers.
- [Security](/docs/features/security) — the controls, and which ones are on by
  default.
