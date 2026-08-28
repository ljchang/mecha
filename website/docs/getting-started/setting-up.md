---
title: Setting up
sidebar_position: 3
description: Point mecha at a model, let it read the settings back off the server rather than typing them, and wire in the personal context you want it to have.
---

# Setting up

Installing gives you the binary. This page gets it to the point where it can
actually answer — a provider it can reach, settings that match what is really
being served, and whatever personal context you want it to have.

If you read one thing here, read [`mecha setup`](#3-mecha-setup) below. Most of
what goes wrong at this stage goes wrong *quietly*, and that command exists to
make the quiet part loud.

## 1. Check the install, before any credentials exist

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

On a fresh install you should see the built-ins: `fs_read`, `fs_write`,
`fs_edit`, `fs_list`, `shell`, `http_fetch`, and `todo`. `web_search` is absent
until a search backend is configured, because a search tool that always errors
is worse than no search tool.

## 2. Point it at a model

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
context_window = 32768             # per slot — not `-c`. See below.
vision = true                      # only if the server has a projector loaded
```

**Do not type those last three by hand.** Each is a setting nothing can check
afterwards, because each degrades quietly rather than failing — which is what
the next step is for.

Then either make it the default:

```toml
default_provider = "local"
```

or select it per run:

```bash
mecha run -p local "..."
export MECHA_PROVIDER=local        # or via the environment
```

## 3. `mecha setup`

```bash
mecha setup
```

It reports what this install still needs and the one command that fixes each.
For a local provider it asks the server what it is *actually* serving and
compares that against your config:

```
! The config disagrees with what is served  [disagrees]
    [providers.local] context_window = 32768, but the server is serving 262144
    tokens per slot. The server has 4 slots and divides `-c` evenly across them,
    so the value to write down is `-c / 4` and not `-c`. …

    [providers.local] is serving a vision model — the projector is loaded and
    paid for in memory — but `vision` is not set, so no image will ever be sent
    to it. Set `vision = true`.

    [providers.local] model = "qwen3-14b", but the server is serving
    "qwen3.6-35b-a3b". llama-server ignores the request's `model` field, so this
    does not change which weights answer — it changes what every session record
    and scorecard says answered.
    → mecha setup --write

· Mail and calendar  [not set up]
    → mecha-mail auth personal --provider google

· Slack as a remote control  [not set up]
    → mecha slack auth

✓ The personal knowledge graph  [ok]

4 step(s) outstanding.
```

At a terminal it then offers each fix as a `y/N`; anything you decline is simply
skipped. `--json` prints the plan and never prompts, and exits 1 while anything
is outstanding, so a script can act on it.

### Letting it write the settings

```bash
mecha setup --write
```

This rewrites `model`, `context_window` and `vision` from what `/props`
reports. It shows you the values first and asks before touching anything, keeps
the previous file as `config.toml.bak`, and edits the table **in place** so your
comments survive.

:::tip Why this is a command and not a paragraph of documentation
All three of these settings were documented correctly for months while the
machine that wrote the documentation had two of them wrong. None of them
*fails* when wrong — a bad `context_window` makes a long run compact at a
threshold nobody chose, an unset `vision` makes every screenshot arrive as a
line of text, and a wrong `model` changes only what your session records claim
answered. Reading them off the wire is the only way to be sure, which is why
the command asks the server instead of asking you.
:::

The same comparison runs at startup on every command, so if the two drift apart
later you hear about it the next time you use mecha at all.

## 4. Wire in personal context

Everything below is optional and independent — mecha is useful with none of it.
`mecha setup` lists whichever you have not done yet, with the command for each.

**And you can say no.** At each offer the answer is `y`, `N`, or `never`:

```text
Watch a run from a phone, approve what it wants to send, and hand files in and out.
run `mecha slack auth`? [y/N/never] never
noted — `slack` will not be offered again (`mecha setup --undecline slack` undoes it)
```

`N` means *not today* and the question comes back; `never` records the choice in
`~/.mecha/setup-declined.json` and the step reads `you said no thanks` from then
on. That is the difference between a finished install and a permanent defect
list — a declined step is **not outstanding**, so `mecha setup` exits 0 and is
usable as your own health check. `mecha setup --undecline <step>` (or `all`)
asks again.

Two things you cannot decline, deliberately: a provider that can answer, and
anything that is *broken* rather than merely absent. "I don't want mail" is a
preference; "stop telling me my mail is broken" is not one a setup tool should
be able to record.

| | What it gives you | Start with |
|---|---|---|
| **Mail and calendar** | Gmail and Outlook behind one surface. The model names an *account*, never a provider. | `mecha-mail auth personal --provider google` |
| **Documents** | Google Docs, Sheets and Slides under `drive.file` — only files it created or you handed it in Google's own picker. | `mecha-docs auth personal` |
| **Slack** | A remote control: watch a run from a phone, approve sends, pass files both ways. | `mecha slack auth` |
| **Knowledge graph** | Memory — who people are, what happened when. A separate project, wired in over MCP. | `cargo install mecha-graph-mcp` |

Two things worth knowing before you start:

- **Mail and documents are separate crates.** `cargo install mecha-mail`
  installs `mecha-mail`, `mecha-docs`, `mecha-google` and `mecha-outlook`.
- **The knowledge graph's own sources** — ambient conversations, a calendar
  feed, messages — are configured with `mecha-graph source`, in that project.
  mecha reaches the graph only through its MCP tools and deliberately knows
  nothing else about it, so `mecha setup` names those sources and never drives
  them.

Reading mail or the graph marks the conversation as holding third-party
content, which is what arms [the trifecta interlock](/docs/features/security).
That is the intended behaviour, not a misconfiguration.

## 5. Write a charter

The one step that is about you rather than about the machine, and the one most
people never find:

```bash
mecha charter edit
```

A charter is a short, **ranked** list of standing priorities in your own words.
It rides in every run's prompt, so it is how mecha knows what you actually want
from it rather than only what you asked for in this sentence.

```toml
[[line]]
id = "tell-the-truth-early"
text = "Tell me the truth early, especially when it disappoints."

[[line]]
id = "protect-my-attention"
text = "Do not put something in front of me that I could not act on today."
```

Order is rank and there is no priority field: when two lines conflict, the
higher one wins outright, and no amount of urgency on a lower line outranks a
higher one. Re-ranking is moving a line.

`mecha charter edit` creates a commented template if you have no file yet and
hands it to `$EDITOR` — the same bytes the TUI's `/charter` and the web settings
page hand out. **mecha never writes a priority.** The template is comments only,
there is no generate button anywhere, and no tool a model can call reaches this
file; a model that could edit its own standing priorities could edit its way
around every other guardrail.

One authoring trap worth knowing before you start: a line shaped like *"never
disappoint anyone"* produces sycophancy and withheld bad news. Point it the
other way, as the first example above does.

See [Goals and appraisal](/docs/features/appraisal) for what the charter is
part of.

## 6. What is deliberately not set up

**Nothing is scheduled for you.** A scheduled unattended agent run on a machine
holding your mail is not something to opt anyone into silently — the same
reasoning that keeps `[[trigger]]` out of a project's `mecha.toml`, where a
cloned repository would be handing itself a cron slot on your machine.

When you do want one, [Triggers](/docs/features/triggers) covers it, and the
runner is a unit you can read before you let it run:

```bash
mecha trigger daemon --print-unit > ~/.config/systemd/user/mecha-triggers.service
```

## Next

- [First run](/docs/getting-started/first-run) — start it and use it
- [Goals and appraisal](/docs/features/appraisal) — the charter, in full
- [Configuration](/docs/getting-started/configuration) — every setting, and what derives from what
- [Images](/docs/features/images) — if you want it to look at screenshots
