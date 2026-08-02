# mecha — handoff

State of the project and what to build next. Written to be picked up cold.

---

## What exists

A working agent harness, used and measured rather than just compiled.

| Area | State |
|---|---|
| Providers | Anthropic (raw HTTP) + OpenAI-compatible (llama-server, vLLM, Ollama) |
| Agent loop | Streaming, tool dispatch, parallel execution, forced final answer |
| Tools | `fs_read/write/edit/list`, `shell`, `http_fetch`, `todo`, `web_search` |
| MCP | stdio client; remote tools become ordinary `Tool` impls |
| Subagents | `Agent` wrapped as a `Tool`, allowlisted registry, per-profile model |
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — with fall-through |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Sessions | Append-only JSONL, resume |
| Eval | 25 cases, 10 tags, scorecard, `--compare` |

36 tests. `cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

Running on the DGX Spark (GB10, aarch64, 128GB unified):

| Port | Model | Notes |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`) |
| 8081 | gemma-4-E4B | separate `mtp-*.gguf` draft |
| 8082 | gemma-4-26B-A4B | separate `mtp-*.gguf` draft |
| 8888 | SearXNG | Docker, JSON format enabled |

Start scripts are in `scripts/` (`start-moe-mtp.sh`, `start-e4b.sh`,
`start-gemma26.sh`).
Config is `~/.mecha/config.toml` (providers `local`, `small`, `gemma26`).

`ANTHROPIC_API_KEY` has never been set in this environment. **No Anthropic
request has ever been made through this code.** The Anthropic provider is
written to spec and compiles, but is unverified against the live API — treat
the first real call as a test, and check streaming, thinking blocks, and cache
breakpoints specifically.

## What the measurements say

All four models score 23–24/25 with zero malformed arguments and zero invented
tools. **The case set has saturated** — it is a floor test, not a ranking test.

Two conclusions hold anyway:

1. **MoE wins on this hardware.** Decode tracks *active* parameters. The dense
   27B is 8× slower than the 3B-active MoE for identical accuracy.
2. **Constrained decoding is doing real work.** `llama-server --jinja`
   grammar-constrains tool calls; that is why malformed-argument counts are
   zero across the board. Don't conclude anything about a model's tool
   reliability from an unconstrained sampler.

Everything in the case set is *grounded* — the data is in the workspace and the
job is to find it, combine it, report. It says nothing about long-horizon
planning, ambiguity, or code generation.

---

## Next features

Ordered by dependency, not by appeal. The first three unlock or gate the rest.

### 1. Harder eval cases

**Why first:** everything downstream is a judgement call until you can measure.
You currently cannot choose a parent model, and cannot tell whether a harness
change helped.

Needs cases that are not grounded lookups:

- **Long-horizon** — a task needing 15+ steps with state carried across them.
- **Ambiguity** — an under-specified request where the right move is to ask, or
  to state an assumption. Grade the *judgement*, not the answer.
- **Code generation** — write a function, run the tests, fix what fails. This
  needs a fixture with a real test command, so `--read-only` has to be
  relaxed for these cases and each one needs an isolated workspace copy.
- **Multi-source synthesis** — combine six documents that partly disagree.

Two known limits of the current rig to fix alongside:

- Cases are forced read-only against a shared fixture. Mutating cases need a
  per-case temp copy — currently `ToolCtx` is per-agent, not per-run, so this
  needs `run()` to take a context or `Agent` to hold `Arc<ToolCtx>`.
- Substring grading won't survive open-ended answers. Add an **LLM-as-judge**
  check type (`expect.judge: "..."`) that runs a rubric against a second model.
  Keep it alongside substring checks, not replacing them — deterministic
  checks are worth more where they apply.

### 2. Interruptibility

**Why before the TUI:** it needs a cancellation token threaded through
`Agent::run` and every provider call. Retrofitting that after a UI is built on
top is much worse than doing it first.

- `tokio_util::sync::CancellationToken` (or a `Notify`) into `run()`.
- Cancel mid-stream: abort the HTTP request, keep the partial assistant turn,
  append a note that it was interrupted.
- Queued input: user types while the agent works; message lands at the next
  turn boundary rather than being dropped.

### 3. Sandboxing

**Why before triggers or email:** `shell` currently runs as you, with your
credentials, and the path jail does not cover it — a `shell` call can `cd`
anywhere. That is acceptable for a supervised CLI and *not* acceptable for an
agent woken by an incoming email.

Options in rough order of effort: bubblewrap/`unshare` namespaces, a Docker
exec into a scratch container, or a full self-hosted sandbox. The capability
model already marks `shell` as private+sends+destructive; this is the missing
enforcement behind that label.

### 4. The remaining surfaces

Roughly independent of each other; all depend on 1–3.

**TUI (ratatui).** Streaming panes, tool-call tree, approval modal, the `todo`
list rendered live. `AgentEvent` already carries everything needed.

**Slack DM.** Socket Mode app in an existing workspace (no new workspace
required). The hard requirement: it must share one session store with the CLI,
or you have two assistants that don't know each other. Decide the identity
model before writing the transport.

**Email / calendar.** Gmail + Graph APIs. **Draft-only, never send** — the
outbox pattern belongs in core as a first-class concept, not as per-tool
politeness. Do not start this before sandboxing and the outbox exist.

**`pkg` as memory.** Wire `pkg-mcp` in as first-class memory: retrieve context
at turn start, stage learnings via `kg_upsert` at turn end, review nightly.
`pkg`'s `fact_candidate` staging queue is exactly the guardrail a self-learning
agent needs — it cannot silently poison its own memory. **Do not build a second
memory store beside it.**

**Triggers.** Cron, file watchers, inbound webhooks. Gated on sandboxing.

### 5. Smaller, high-value items

- **Hooks** — pre-tool / post-tool / session lifecycle. Lets policy, redaction,
  and logging attach without touching the loop.
- **Trajectory replay** — re-run a saved session against a different model or
  harness version and diff. Turns every past session into a regression test.
  Sessions are already JSONL; this is mostly a driver.
- **Structured-output abstraction** — a `structured_output` knob on `Provider`
  that each backend spells natively (GBNF for llama.cpp, `guided_json` for
  vLLM, `output_config.format` for Anthropic). Don't hardcode GBNF.
- **Context compaction** — long sessions will blow the window. Anthropic has
  server-side compaction; local models need client-side summarization.
- **Phase-gated tools** — a state machine where planning cannot call write
  tools. Structural, so it can't be prompted away.

---

## Conventions worth keeping

- **The loop never learns where a tool came from or which provider is behind
  it.** Both are trait objects. Matching on provider name inside `agent.rs`
  means the abstraction is leaking.
- Tools return `Ok(ToolOutput { is_error: true })` for expected failures so the
  model can recover. Reserve `Err` for what it cannot route around.
- Write tool error messages **for the model**. "not found; the directory
  contains a.md, b.md" is a self-correcting loop; "No such file" is a dead end.
- Every model-supplied path goes through `ToolCtx::resolve`. Never `fs::*` on
  raw tool input.
- A tool result must exist for every `tool_use` id or the next request 400s.
- `Capabilities::untrusted_input` is what a tool *can* return;
  `ToolOutput::external` is where this result *actually came from*. Taint keys
  off provenance. Any tool reaching the network calls `.from_outside()`.
- Registry order is stable (`BTreeMap`) because the tool list is the front of
  the cached prefix.

## Traps already hit

Recorded so they aren't hit twice:

- **A wrong gold answer measures nothing.** One was shipped ($2,450 vs the
  correct $1,750) by double-counting a base rate. Verify arithmetic with a
  script.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right.
  `eval::normalize` handles it now — extend that, don't work around it.
- **Wrapping our own errors as untrusted content** made the model invent
  explanations for its own harness's behaviour. Provenance, not capability.
- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
- **`hf download repo --include X Y`** silently ignores `--include` when
  positional filenames are given. Pass filenames positionally *or* use
  `--include`, not both.
- Free-tier claims in comparison articles are often stale. Exa's own page says
  $10/month recurring credits (~1,400 searches), not the 20,000 some
  aggregators report.
