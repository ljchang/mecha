# mecha

A standalone agent harness. One loop, any model, native and MCP tools —
usable as a CLI, as a library, and as a batch runner.

The point is to stop rewriting the same harness in every project. `mecha-core`
is a plain Rust library that knows nothing about any particular application;
`mecha` is a thin CLI over it.

```
        PROVIDERS                    mecha-core                    SURFACES
  ┌──────────────────────┐    ┌────────────────────────┐    ┌──────────────────┐
  │ Anthropic (Claude)   │    │  agent  — the loop     │    │ mecha run        │
  │ OpenAI-compatible ───┼───▶│  tool   — registry     │───▶│ mecha chat       │
  │   · llama-server     │    │  mcp    — stdio client │    │ mecha batch      │
  │   · vLLM / Ollama    │    │  session — transcripts │    │ your crate       │
  └──────────────────────┘    └────────────────────────┘    └──────────────────┘
```

## Install

```bash
cargo build --release          # binary at ./target/release/mecha
```

## Quick start

```bash
export ANTHROPIC_API_KEY=sk-ant-...
mecha config init              # writes ~/.mecha/config.toml

mecha run "summarize what changed in this repo today"
mecha chat                     # interactive
mecha tools                    # what the agent can do (no credentials needed)
```

By default the agent may read anything in the workspace but asks before it
writes or runs a command. `--yes` approves everything; `--read-only` refuses
everything that isn't a read.

## Commands

| Command | What it does |
|---|---|
| `mecha run "<task>"` | One task, one answer. `--json` for machine-readable output, `--resume <id>` to continue. |
| `mecha chat` | Terminal REPL with history and slash commands. |
| `mecha batch items.jsonl` | Same agent over many prompts, bounded concurrency, JSONL results. |
| `mecha eval [cases.jsonl]` | Score a model on a case set. The bake-off rig — see below. |
| `mecha tools` | List the tool surface. `--schema` shows exactly what the model sees. |
| `mecha sessions list\|show\|path` | Inspect saved transcripts. |
| `mecha config show\|path\|init` | See what settings are in effect. |

Exit codes for `run`: `0` success, `1` error, `2` the model refused, `3` it ran
out of turns.

## Configuration

Layered, each level overriding only the fields it names:

1. built-in defaults
2. `~/.mecha/config.toml`
3. `./mecha.toml` (project-local)
4. `MECHA_PROVIDER` / `MECHA_MODEL` / `MECHA_EFFORT`
5. CLI flags

```toml
default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"

[providers.local]                     # llama-server, vLLM, Ollama
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "qwen3-14b"

[agent]
max_turns = 40
max_tokens = 64000
effort = "high"                       # low | medium | high | xhigh | max
thinking = true
cache_prompt = true

[tools]
permission_mode = "ask"               # ask | allow | read-only
shell_timeout_secs = 120

[[mcp]]
name = "pkg"
command = "~/Github/personalized_knowledge_graph/target/release/pkg-mcp"
```

## Tools

Built in: `fs_read`, `fs_write`, `fs_edit`, `fs_list`, `shell`, `http_fetch`.
All filesystem paths are resolved and checked against the workspace root before
anything touches disk — `..`, symlinks, and absolute paths outside the root are
refused.

Anything else comes from MCP. Each server's tools are namespaced
`<server>__<tool>`, so two servers can both expose a `search`. MCP tools and
built-ins are the same trait to the agent loop.

## As a library

```rust
use mecha_core::{agent::Agent, config::Config, message::Message};
use mecha_core::tool::{ModeApprover, Registry, ToolCtx};
use std::sync::Arc;

let cfg = Config::load(&std::env::current_dir()?)?;
let (_, provider_cfg) = cfg.provider(None)?;

let agent = Agent::new(
    mecha_core::provider::build(provider_cfg)?,
    Registry::new().with_builtins(&cfg.tools),
    Arc::new(ModeApprover { mode: cfg.tools.permission_mode }),
    ToolCtx {
        workspace: std::env::current_dir()?,
        shell_timeout: std::time::Duration::from_secs(120),
    },
    cfg.agent.clone(),
    None,
)?;

let mut messages = vec![Message::user("what changed today?")];
let outcome = agent.run(&mut messages, None).await?;
```

Implement `Tool` to add a native tool, `Provider` to add a backend, and
`Approver` to control what needs permission.

## Batch

```bash
# items.jsonl — one object per line, or a bare JSON string
{"id": "q1", "prompt": "who did I meet with last week?", "meta": {"gold": "..."}}

mecha batch items.jsonl --concurrency 8 --out results.jsonl --yes
```

Results stream to the output file as they finish, keyed by `id` — a killed run
still leaves everything completed so far on disk.

## Choosing a model (`mecha eval`)

The hard part of running locally isn't capability, it's **tool-call
reliability**: a model that is 5% smarter but malforms JSON arguments 1-in-20
calls is worse in a loop, because every bad call costs a recovery turn. So
`mecha eval` grades the **tool-call trace**, not just the final text.

```bash
mecha eval -p local -m qwen3-moe   -o results/qwen.json
mecha eval -p local -m nemotron    -o results/nemotron.json
mecha eval -p anthropic            -o results/opus5.json     # the ceiling

mecha eval --compare results/*.json
```

Runs are forced read-only against `eval/workspace`, so they're reproducible,
safe at high concurrency, and comparable across models.

Cases are JSONL, graded on what the model *did*:

```json
{"id":"list-then-read","tags":["chaining"],
 "prompt":"Look at what is in the notes directory, then read the earliest note and tell me who attended.",
 "expect":{"tools_in_order":["fs_list","fs_read"],"contains":["wasita"],"max_turns":6}}
```

| Expectation | Checks |
|---|---|
| `tools` | each named tool was called at least once |
| `tools_in_order` | called in this relative order (interleaving allowed) |
| `forbid_tools` / `no_tools` | never called / no tool used at all |
| `args` | a call to that tool passed an argument matching `equals`/`contains` |
| `contains` / `not_contains` / `contains_any` | substrings of the final answer |
| `max_turns` | the run didn't flail |

Two checks are applied to every case whether you ask for them or not, because
they disqualify a model regardless of the answer: **malformed arguments** and
**invented tool names**.

The shipped set covers single calls, chaining, argument fidelity, tool
selection among distractors, discrimination (knowing *not* to use a tool),
recovery from errors and denials, and honesty about missing capabilities.
`--tag chaining` runs one slice; `--failures` shows why each case failed.

`mecha eval` exits non-zero when anything fails, so it also works as a
regression gate on the harness itself.

## Sessions

Every run writes an append-only JSONL transcript to `~/.mecha/sessions`
(override with `MECHA_SESSION_DIR`). `--no-session` opts out.
