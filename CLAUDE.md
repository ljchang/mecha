# mecha — CLAUDE.md

## What this is

A standalone agent harness, extracted so it can be reused across projects
rather than rewritten per project. Two crates:

- `mecha-core/` — the library. Knows nothing about any CLI or application.
- `mecha-cli/` — the `mecha` binary. Thin; all logic belongs in core.

Interfaces: `mecha run` (one-shot), `mecha chat` (readline REPL), `mecha tui`
(full-screen; the input line stays live so you can steer a run in flight), and
`mecha batch` / `mecha eval` for fan-out.

## Build & test

```bash
cargo build --release          # ./target/release/mecha
cargo test                     # unit tests, incl. a scripted-provider loop test
cargo clippy --all-targets
```

`MECHA_LOG=debug` turns on internal tracing (goes to stderr).

## Architecture

```
message.rs   provider-agnostic Message/Block/Usage/StopReason types
provider/    Provider trait + anthropic.rs (raw HTTP) + openai.rs (compatible)
tool/        Tool trait, Registry, Approver, builtin.rs
mcp.rs       stdio JSON-RPC client; wraps remote tools as Tool impls
agent.rs     the loop: ask → run tools → feed results back → repeat
session.rs   append-only JSONL transcripts
batch.rs     bounded-concurrency fan-out over many prompts
eval.rs      case types, graders, the LLM judge
config.rs    layered TOML config
```

`RunContext` is what one *run* gets: the path jail, the approver, its budget,
and optionally a cancellation token and a steering queue.
`Agent::run` uses the agent's own; `Agent::run_in` takes a caller's.
That is what lets one agent — one provider connection, one cached prefix —
serve concurrent runs jailed to different directories under different
permissions, which is how a mutating eval case gets a private workspace while
the case beside it stays read-only.

## Interruption and steering

Two different things, and the difference is the point:

- **Cancel** (`RunContext::cancel`) stops the run at the next safe point — a
  turn boundary or mid-stream — and keeps the partial turn. Cancellation is a
  dropped future; that is what aborts the HTTP request. A cancellable run
  always streams, because otherwise there is no partial answer to keep. Tools
  are never interrupted mid-call.
- **Steer** (`RunContext::queued_input`) redirects a run *without* stopping it.
  Text queued mid-run is folded into the message carrying the tool results, so
  the model sees the results and the new instruction as one user turn and keeps
  going. Never append a bare user message instead: two user messages in a row
  are invalid, and there is no legal slot between a `tool_use` and its result.

Only `mecha tui` can steer, and that is a property of the front-end, not the
loop: steering needs one owner of stdin, which a readline REPL cannot be while
a run is streaming. Testing the TUI means driving a pty — and giving it a size
(`script -qec "stty rows 45 cols 130; mecha tui" /dev/null`), because a pty
with no window size renders every frame into a 0x0 area.

The invariant worth protecting: **the agent loop never learns where a tool came
from or which provider is behind it.** Both are trait objects. If you find
yourself matching on provider name inside `agent.rs`, the abstraction is
leaking.

## Provider notes (Claude 5 family)

There is no official Anthropic SDK for Rust, so `provider/anthropic.rs` speaks
raw HTTP. Things that will 400 if forgotten:

- `temperature` / `top_p` / `top_k` are **rejected** — never send them.
- `budget_tokens` is gone; thinking is `{"type": "adaptive"}`.
- On Opus 5, thinking is **on by default**, and `{"type": "disabled"}` is only
  accepted at effort `high` or below. `Anthropic::body` fails early with a
  useful message rather than letting the API reject it.
- `stop_reason: "refusal"` arrives as **HTTP 200**. Always check the stop reason
  before reading content.
- Prompt caching is a prefix match: render order is tools → system → messages,
  so the `cache_control` breakpoint goes on the last system block and covers the
  tools too. Anything volatile must come after it.

Model IDs are exact strings with no date suffix (`claude-opus-5`).

## Security model

Two things are enforced structurally rather than by prompting:

**The path jail.** Every model-supplied path goes through `ToolCtx::resolve`,
which canonicalizes and proves containment in the workspace. Never call `fs::*`
on a raw path from tool input.

**The trifecta interlock.** Taint is a property of the **conversation**, not of
one run — it lives on `agent::Conversation` alongside the messages, and the
session file records it so resuming does not launder it. It used to be created
fresh inside `run`, which meant a chat turn reset it: fetch a hostile page on
turn one, read a secret and send on turn two, and the interlock saw a clean
slate both times while the attacker's text sat in context the whole while. A
turn boundary is not a security boundary. Bundling taint with the messages makes
the right thing the default — keep the history and you keep the taint; start a
new `Conversation` (a batch item, a subagent, an eval case) and you get a clean
one.

Tools declare `Capabilities` — `private_data`,
`untrusted_input`, `external_send`, `destructive`. The loop tracks which have
entered the conversation (`Taint`) and refuses any `external_send` tool once
both private and untrusted are present. It sits *ahead* of the approver on
purpose: a human clicking "yes" is what an injection is trying to engineer.

Two distinctions that are easy to get wrong:

- `Capabilities::untrusted_input` says what a tool *can* return.
  `ToolOutput::external` says whether this particular result actually came from
  outside. Taint and untrusted-marking key off **`external`**, not the
  capability — otherwise a refusal generated by our own guard gets labelled as
  third-party content and the model invents explanations for its own harness.
  Any tool that reaches the network must call `.from_outside()`.
- `http_fetch` is `read_only` (it doesn't touch your data) but is still an
  `external_send` sink, because the payload fits in a query string.

Known gap: `shell` is universal and taint tracking can't see inside a command,
so it is not treated as an untrusted *source*. The mitigation is the sandbox.

**The sandbox** (`sandbox.rs`, `[sandbox]` in config) is the enforcement behind
`shell`'s capability label. `kind = "bwrap"` uses user namespaces; `"docker"`
runs a throwaway container; `"none"` (the default) runs commands as you. A
confined command gets the workspace, a read-only system, no home directory, no
environment except a named allowlist, and by default no network.

Three rules here, each of which cost something to learn:

- **A configured sandbox that doesn't work stops the run.** `Sandbox::preflight`
  runs a real command through the real backend at startup and fails with
  instructions. Silently falling back to unconfined execution is worse than no
  sandbox, because `shell` declares narrower capabilities when confined and the
  interlock believes it.
- **Only `external_send` narrows.** No network means no way out, so a confined
  shell stops being a trifecta sink. `private_data` stays true: a confined shell
  still reads the workspace, and `fs_read` is marked private for exactly those
  bytes. Narrowing it would make `shell: cat secrets` set no taint where
  `fs_read` does — the cheapest route around the interlock must never be the
  more dangerous tool.
- **The policy lives on the tool, not in `ToolCtx`.** `capabilities()` has no
  context to consult. The workspace still comes from the context at call time,
  so a per-run jail (an eval case's private fixture copy) is what gets mounted.

**MCP servers get the same treatment**, and they need it more: an MCP server is
third-party code running on your machine, where `shell` at least runs commands a
model asked for out loud. Two rules:

- **The environment is an allowlist, not an inheritance.** `Command::envs()`
  adds to the inherited environment rather than replacing it, which is how a
  server ends up holding your provider keys without anyone deciding it should.
  `connect` clears first, then adds a minimal base (`PATH`, `HOME`, `LANG`,
  `LC_ALL`, `TZ`) plus whatever `env_passthrough` names and `env` sets.
  Measured on a deliberately nosy test server: 64 variables including two API
  keys, down to 3 and none.
- **`sandbox = true` on a server that cannot be confined is an error**, not a
  warning — the same rule as `shell`, for the same reason. Per-server `network`
  overrides the global switch, because otherwise you would have to give `shell`
  the network to let one server reach its own API.

On Ubuntu 23.10+, `bwrap` fails even when installed and
`unprivileged_userns_clone=1`, because AppArmor gained a separate switch:
`kernel.apparmor_restrict_unprivileged_userns=1`. Use `docker` there, or install
an AppArmor profile. `mecha tools` prints the active sandbox, and
`mecha tools --json` prints each tool's capabilities.

## Conventions

- Tools return `Ok(ToolOutput { is_error: true })` for expected failures — the
  model can then recover. Reserve `Err` for things it can't route around.
- Every model-supplied path goes through `ToolCtx::resolve` before touching the
  filesystem. Never call `fs::*` on a raw path from tool input.
- Approval is sequential (it may block on a human); execution is concurrent.
- A tool result must exist for every `tool_use` id, or the next request 400s.
- Tool order in the registry is stable (`BTreeMap`) because the tool list is the
  front of the cached prefix — reordering it invalidates the cache every turn.

## The eval rig

`eval/cases.jsonl` is graded on the tool-call trace first, text second. Four
kinds of check, in descending order of how much they are worth:

- **Trace and substring checks** — deterministic, free, and they never change
  their mind. Use them wherever they apply.
- **`expect.verify`** — a command run in the case's workspace afterwards,
  passing iff it exits 0. The ground truth for codegen: not whether the model
  said the tests pass, but whether they do. Requires `sandbox`.
- **`expect.judge`** — a rubric graded by a second model, for cases where the
  right answer is a judgement. Not deterministic: the same answer can be graded
  differently across runs, so treat a single judge failure as a prompt to read
  the answer, not as a result.
- Everything a model says about its own work is hearsay. Grade the artifact.

Two things a case can ask for beyond the defaults:

- `"sandbox": true` — a private copy of the fixture, with writes allowed.
  Required for `verify`. The shared fixture is never mutated.
- `"max_turns": N` — a per-case turn budget. A case that genuinely takes twenty
  steps says so, rather than everyone raising the global ceiling for one case
  and quietly changing what every other case may do.

Fixtures under `eval/workspace/{audit,reports,kata}` are generated:
`python3 scripts/build-eval-fixtures.py` rewrites them, prints the gold answers
the cases must assert, and checks that each kata fails as shipped *and* is
solvable by a reference fix. A gold answer typed by hand is a guess, and a
wrong one measures nothing.

## Testing without credentials

`agent.rs` has a `ScriptedProvider` that replays a fixed list of turns. Use it
to test loop behavior (tool dispatch, denials, exhaustion, error recovery)
without network access. `mecha tools` also runs without any provider configured,
which makes it a good MCP-server smoke test.
