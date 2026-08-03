# mecha — handoff

State of the project and what to build next. Written to be picked up cold.

---

## Where the work is

**Branch `harden-and-measure`, 8 commits ahead of `main`, nothing merged.** The
working tree is clean. `main` is still at `40ce7a7`.

```
c7a5813  Record that the Anthropic provider is now verified against the live API
ba5c6cb  Compact the transcript so long sessions keep fitting
a100616  Make taint a property of the conversation, not of one run
d6b24ed  Update CLAUDE.md and the handoff for what these four commits changed
2c898ac  Add `mecha tui`, where the input line stays live while the agent works
7c1a02b  Harder eval cases: sandboxed workspaces, real test runs, an LLM judge
857f47d  Give each run its own context, and let it be interrupted or steered
7b40aa7  Confine `shell` and MCP servers instead of only labelling them
```

Each commit was verified to build and pass tests **in isolation** (stash the
rest, build, test, commit), so the history bisects rather than merely ending in
a good state. Build order forced the sandbox commit first even though it was
written last.

**Decide whether to fast-forward `main` onto this branch.** The repo's prior
history commits straight to `main`; the branch exists because the default
guidance is not to commit to a default branch unasked. Nothing depends on the
branch staying separate.

First thing to run in a fresh context:

```bash
cargo test && cargo clippy --all-targets -- -D warnings   # 72 tests, no warnings
```

## What exists

A working agent harness, used and measured rather than just compiled.

| Area | State |
|---|---|
| Providers | Anthropic (raw HTTP, **verified live**) + OpenAI-compatible (llama-server, vLLM, Ollama) |
| Agent loop | Streaming, tool dispatch, parallel execution, forced final answer |
| Tools | `fs_read/write/edit/list`, `shell`, `http_fetch`, `todo`, `web_search` |
| MCP | stdio client; remote tools become ordinary `Tool` impls |
| Subagents | `Agent` wrapped as a `Tool`, allowlisted registry, per-profile model |
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — with fall-through |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Sandbox | `shell` and MCP servers confined via bubblewrap or docker; no network by default |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Context | Compaction with tool-call-safe cut points, taint preserved |
| Interfaces | `run`, `chat`, `tui` (live input line, steer while it works), `batch`, `eval` |
| Sessions | Append-only JSONL, resume, taint recorded |
| Eval | 34 cases, 15 tags, scorecard, `--compare`, sandboxes, verify commands, LLM judge |

72 tests. `cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

Running on the DGX Spark (GB10, aarch64, 128GB unified):

| Port | Model | Notes |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`) |
| 8081 | gemma-4-E4B | separate `mtp-*.gguf` draft — **was down at last check** |
| 8082 | gemma-4-26B-A4B | separate `mtp-*.gguf` draft; used as the eval judge |
| 8888 | SearXNG | Docker, JSON format enabled |

Start scripts are in `scripts/` (`start-moe-mtp.sh`, `start-e4b.sh`,
`start-gemma26.sh`). Config is `~/.mecha/config.toml` (providers `local`,
`small`, `gemma26`, `anthropic`).

### The Anthropic key

`ANTHROPIC_API_KEY` is set, in `~/.bashrc`. Two gotchas, both of which cost
time:

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard around line 5). The export is well below it, so a non-interactive shell
  — which is what tooling runs — never reaches it. Load it explicitly:
  `eval "$(grep '^export ANTHROPIC_API_KEY' ~/.bashrc | tail -1)"`.
- **Take the *last* match, not the first.** There were two exports for a while,
  a placeholder above the real key; `grep -m1` silently found the placeholder
  and produced a `401 invalid x-api-key` that looked like a bad key.

### The Anthropic provider is verified

As of 2026-08-03, which it had never been before — ~480 lines written to spec
with no evidence behind them. Five checks, all passing:

| Check | Result |
|---|---|
| Plain call | works |
| Prompt cache | 3398 tokens written, then **3398 read** on the next call — the breakpoint placement (tools → system → messages, marker on the last system block) is right |
| Tool round-trip | works, cache hit on both turns |
| Thinking blocks across a tool turn (Opus 5) | signatures echo back correctly — a wrong signature would 400 on turn two |
| Ctrl-C mid-stream | cut off cleanly at 289, partial kept, exit 3 |

Still unverified: the refusal path (`stop_reason: "refusal"` arriving at HTTP
200) — not worth deliberately eliciting.

**No prices are configured**, so `cost_usd` is `None` and `--max-cost` silently
never fires on a paid provider. For `[providers.anthropic]`:

```toml
input_price_per_mtok = 5.0     # claude-opus-5
output_price_per_mtok = 25.0
```

Sonnet 5 is $3/$15 ($2/$10 introductory through 2026-08-31). We send
`cache_control` with no `ttl`, so the 5-minute tier applies and mecha's default
multipliers (1.25 write, 0.1 read) are already correct.

## What the measurements say

On the original 25 grounded cases, all four local models score 23–24/25 with
zero malformed arguments and zero invented tools. **That set saturated** — it is
a floor test, not a ranking test, and it stays in the file as exactly that.

Two conclusions hold from it anyway:

1. **MoE wins on this hardware.** Decode tracks *active* parameters. The dense
   27B is 8× slower than the 3B-active MoE for identical accuracy.
2. **Constrained decoding is doing real work.** `llama-server --jinja`
   grammar-constrains tool calls; that is why malformed-argument counts are
   zero across the board. Don't conclude anything about a model's tool
   reliability from an unconstrained sampler.

The nine cases added since (`long-horizon`, `codegen`, `synthesis`,
`ambiguity`) do discriminate. qwen3.6-35b-a3b judged by gemma-4-26b-a4b scores
**32/34** on the full set (`results/qwen-hard-v2.json`), and both failures are
in the same tag:

- **long-horizon 2/2**, at ~17.5 turns — it walks a 16-link chain without
  losing the running total, and does not take the shortcut of summing the
  decoys.
- **codegen 2/2** — implements `median`, finds the one-line duration-parsing
  bug, and runs the tests itself. Graded by running them, not by asking.
- **synthesis 2/2** — finds the majority figure and the outlier, and notices
  which report supersedes which.
- **ambiguity 1/3** — the weak spot, and the one that moves between runs
  (1–2/3 across four runs). Given "add the new contractor at the usual rate" it
  sometimes asks, and sometimes spends its whole budget hunting for a
  contractor that does not exist. That variance is itself the finding, and it
  is the case to watch when comparing models.

Only `ambiguity` and `synthesis` have a judge in the loop, and judges disagree
with themselves across runs. Read the answer before believing a single verdict.

**Scorecards in `results/` from before this change are not comparable to ones
after it.** The new fixtures took the shared workspace from 11 files to 44, so
every case that searches the whole workspace got harder — two of them started
failing on turn ceilings calibrated against the smaller tree. If you add
fixtures, expect to recalibrate, and re-baseline every model rather than
comparing across the boundary.

---

## What to do next

The three items that used to gate everything — measurement, interruptibility,
sandboxing — are done. Their design notes are further down, kept as reference
rather than as a checklist. What follows is genuinely independent; pick by what
you want to use.

### 1. Trajectory replay — the agreed next job

Re-run a saved session against a different model or harness version and diff.
Sessions are already JSONL, so this is mostly a driver.

Why it is worth doing before the other surfaces: this project's case set has
**saturated once already**, and the replacement took a full session of
hand-writing cases, four of whose graders were wrong before they were right.
Replay turns every real session into a regression case for free — the scalable
version of what was done by hand.

What the driver has to decide, none of which is obvious:

- **What "the same" means.** A replayed run will not reproduce the original
  tool results (files changed, the web moved). Either replay against a staged
  workspace, or replay the *recorded* tool results and only compare the model's
  choices — the second is closer to a regression test and much cheaper.
- **What to diff.** Final text is the weakest signal, exactly as in the eval
  rig. Diff the tool-call trace first: which tools, in what order, with what
  arguments. `ToolCallTrace` is already recorded and already serialised.
- **Where it lives.** It is the same shape as `mecha eval` — load cases, fan
  out, grade, scorecard — so most of `eval.rs` and `batch.rs` should be reused
  rather than duplicated. A replayed session is an `EvalCase` whose expectations
  are derived from the recorded run.

### 2. Smaller, high-value items

- **Hooks** — pre-tool / post-tool / session lifecycle. Lets policy, redaction,
  and logging attach without touching the loop.
- **Structured-output abstraction** — a `structured_output` knob on `Provider`
  that each backend spells natively (GBNF for llama.cpp, `guided_json` for
  vLLM, `output_config.format` for Anthropic). Don't hardcode GBNF.
- **Phase-gated tools** — a state machine where planning cannot call write
  tools. Structural, so it can't be prompted away.
- **pass@k in the eval** — cases are graded per-run, so a flaky judge or a
  borderline case shows up as noise. Running each case k times would cost k×
  and is worth it for the `ambiguity` tag specifically.
- **`context_window` on `ProviderConfig`** — the compaction threshold is an
  absolute token count because nothing here knows any model's window. Would let
  it be a fraction, and wants the same treatment as pricing: configured, never
  guessed.
- **Usage on an interrupted run** — reports **zero**, because token counts
  arrive in the final SSE frame that never comes. The tokens were spent.
  Estimate them or report unknown, but not zero.

### 3. The remaining surfaces

Roughly independent of each other.

**Slack DM.** Socket Mode app in an existing workspace (no new workspace
required). The hard requirement: it must share one session store with the CLI,
or you have two assistants that don't know each other. Decide the identity
model before writing the transport.

**Email / calendar.** Gmail + Graph APIs. **Draft-only, never send** — the
outbox pattern belongs in core as a first-class concept, not as per-tool
politeness. Do not start this before the outbox exists.

**`pkg` as memory.** Wire `pkg-mcp` in as first-class memory: retrieve context
at turn start, stage learnings via `kg_upsert` at turn end, review nightly.
`pkg`'s `fact_candidate` staging queue is exactly the guardrail a self-learning
agent needs — it cannot silently poison its own memory. **Do not build a second
memory store beside it.**

**Triggers.** Cron, file watchers, inbound webhooks. Sandboxing exists now, so
this is unblocked.

**TUI polish.** The `todo` list is not a live pane, and nested subagent calls
render flat rather than as a tool-call tree.

### 4. Open security gaps

- **A confined MCP server sees the workspace**, so a filesystem server confined
  this way is confined against your home directory, not against your project.
  The right trade for most servers, and worth knowing.
- **HTTP/SSE MCP transports** are not implemented, and when they are, none of
  the process confinement applies — there is no child process, and the trust
  question moves to the endpoint.
- **Subagent workspaces.** A subagent inherits the caller's workspace; there is
  no way to give a child a *narrower* jail than its parent, which is the
  natural next capability restriction.
- **`shell` is enabled inside sandboxed eval cases**, because a codegen case has
  to run its tests. The staged workspace is a copy, so the *fixture* is safe,
  but the command still runs as you — the case file is trusted input.

---

## Design notes on what is already built

Not a checklist. These are the decisions that were expensive to reach and would
be easy to undo by accident.

### Per-run context

`RunContext` carries what is properly per-*run* rather than per-agent: the path
jail, the approver, the budget, and optionally a cancellation token and a
steering queue. `Agent::run_in` takes one; `Agent::run` uses the agent's own.
One agent — one provider connection, one cached prefix — therefore serves
concurrent runs jailed to different directories under different permissions.

Subagents inherit the *caller's* workspace rather than the one that existed when
they were built. That also closed a hole: a parent in a sandbox used to delegate
to a child still pointed at the original directory.

### The eval rig

`"sandbox": true` stages a private fixture copy per case and allows writes;
`"max_turns": N` gives a case its own turn budget; `expect.verify` runs a
command in the case's workspace afterwards and grades the exit code;
`expect.judge` grades a rubric with a second model (`--judge-provider`,
`--judge-model`).

- **Grade the artifact, not the claim.** For codegen, `verify` runs the tests.
  The command hashes the test file first, so a model that edits the tests until
  they pass fails.
- **A judge that cannot answer must fail the case, never skip it.** A case whose
  only real assertion silently evaporates is worse than one that fails loudly.
- **Fixtures are generated** (`scripts/build-eval-fixtures.py`), which is how
  the gold answers get computed rather than guessed, and how the katas are
  checked to fail-as-shipped *and* be solvable.

### Interruption and steering

**Cancel** (`RunContext::cancel`) stops the run at the next safe point and keeps
what it has: `StopCause::Interrupted`, partial text marked incomplete, and the
partial assistant turn appended so the session resumes from the cut.

- **Cancellation is a dropped future.** `Agent::complete` selects between the
  provider call and the token; losing the race drops the provider future, which
  aborts the in-flight HTTP request. There is nothing else to abort.
- **A cancellable run streams**, even when nobody is watching, because that is
  the only way to have the half-written answer when the request is dropped.
  This is why the field is opt-in: a batch worker nobody can interrupt should
  not silently change transport.
- **Tools run to completion.** The cancellation points are the turn boundary and
  mid-stream, not mid-tool. Interrupting a `shell` half way through a write is
  worse than waiting for it.

**Steer** (`RunContext::queued_input`) redirects a run *without* stopping it.
Text typed mid-run is drained at the top of each turn and folded into the message
that already carries the tool results, so the model reads "here is what your
tools returned, and also: actually, focus on X" as one user turn and keeps
working. That placement is forced and also correct: between an assistant's
`tool_use` and its results there is no legal slot for a user message, so the
results message is the first opening. Both encoders preserve it (Anthropic as a
second content block, OpenAI as a trailing `role: "user"` after the `role:
"tool"` ones).

`mecha tui` is the only front-end that can steer. `mecha chat` deliberately
cannot: a readline REPL cannot read stdin while a run streams without a second
reader on the same descriptor, and whichever is blocked when the run ends steals
the user's next prompt line.

### The TUI

One event loop owns the terminal for the session; the agent runs in a task
beside it. Enter starts a run when idle and *steers* one already going. Ctrl-C
cancels the run rather than killing the process; Ctrl-C again at an idle prompt
quits. Approval is a modal, because the terminal approver's `read_line` would
fight the event loop for stdin and its prompt would tear the frame —
`setup::prepare_with_approver` exists for that and only swaps the approver in
`Ask` mode.

The steering case that matters, from a real run:

```
● shell  sleep 6 && echo one
● shell  sleep 6 && echo two
● shell  sleep 6 && echo three
↳ change of plan: skip the rest and just reply with the single word PIVOT  (steering)
PIVOT
```

The fourth command never ran, and the run was never stopped and restarted.

**Testing a TUI needs a pty *with a window size*.** `script -qec "stty rows 45
cols 130; mecha tui" /dev/null`. Without the `stty`, every frame renders into a
0×0 area and the output is pure escape sequences — it looks exactly like a
broken program.

### Sandboxing

`[sandbox] kind = "bwrap" | "docker" | "none"`. A confined command gets the
workspace, a read-only system, no home, no environment beyond a named allowlist,
and by default no network. Default is `none`, because turning it on cannot be
right for every machine — but `mecha tools` says so out loud.

Verified end-to-end through the agent on the docker backend: uid 1000, `~/.ssh`
absent, container hostname, 6 environment variables, DNS dead, and files written
into the workspace owned by the user rather than root (`--user`, without which
the agent leaves root-owned files you cannot delete).

- **A broken sandbox stops the run.** `preflight` runs a real command through
  the real backend at startup. Falling back to unconfined execution would be
  worse than never configuring one, because `shell`'s declared capabilities
  narrow when confined and the interlock trusts them.
- **Only `external_send` narrows**; `private_data` stays true. A confined shell
  still reads the workspace, and `fs_read` is private for exactly those bytes.
  This was wrong first: narrowing it would have made `shell: cat secrets` set no
  taint where `fs_read: secrets` does, so the cheapest way around the interlock
  would have been the more dangerous tool. There is a test named after the hole.
- **`bwrap` does not work on this machine.** Installed,
  `unprivileged_userns_clone=1`, and still fails with `setting up uid map:
  Permission denied`, because Ubuntu 23.10+ added
  `kernel.apparmor_restrict_unprivileged_userns=1` and ships no AppArmor profile
  for bwrap. The docker backend exists because of this, and the error message
  says all of that when it fires.

**MCP servers are covered too**, and were the bigger hole: third-party code, not
commands a model asked for out loud.

- `env_passthrough` replaced inheritance. A nosy test server went from 64
  variables including two API keys to 3 and none. This is a **breaking change**
  for any server that relied on inheriting a token — name it in
  `env_passthrough` or set it in `env`.
- `sandbox = true` per server confines it with the global backend, and
  per-server `network` overrides the global switch, because otherwise reaching
  one server's API would mean giving `shell` the network.
- Asking for confinement with no backend set is a **startup error**, not a
  warning. `mecha tools` lists every server and says which are unconfined.

### Taint is per-conversation

It used to be created fresh inside `run`, so a chat turn reset it and the lethal
trifecta was defeated by pressing Enter. It now lives on `agent::Conversation`
with the messages, and is written to the session file — provenance cannot be
recovered by reading a transcript back, so without that record, resuming
laundered it too. Verified across a process restart: a page fetched in one
session, a file read in the resumed one, and the outbound call refused.

The type is the fix. A caller that keeps the history keeps the taint; one that
starts a new `Conversation` gets a clean one, which is why batch items,
subagents and eval cases do not contaminate each other. The regression test was
checked to **fail against the old behaviour**, not merely pass against the new.

### Compaction

`[agent] compact_at_tokens` / `--compact-at`, off by default because it is
lossy. Verified against llama-server on the 16-link audit chain at a 1200-token
threshold: it compacted four times and still answered 16 entries / 847, matching
the gold. The summaries carried the running total forward, which is the part
that could have destroyed the task silently.

- **The cut has to be legal, not convenient.** A `tool_result` whose `tool_use`
  is gone is a 400, and that is the whole run. Tool results arrive in the user
  message right after the turn that asked for them, so the only safe place to
  resume is an assistant message. `compact.rs` is pure and provider-free for
  exactly this reason; the loop re-checks the rebuilt transcript and refuses to
  install it *before* assigning, since an error here means "carry on
  uncompacted".
- **The summariser gets prose, not a replay.** Sending the real messages means
  sending `tool_result`s on a request that declares no tools, and llama-server
  answers that with an empty completion. Found by running it, not by reading a
  spec — assume it is true of other providers too.
- **Taint survives compaction.** Summarising away the text of a hostile page
  does not un-read it. Taint lives on `Conversation`, which the compaction code
  never touches, so the type does the work — but there is a test, because it is
  the invariant that would be easiest to break later.

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
- Verify a fix by making it **fail on the old behaviour**, not just pass on the
  new one. Two bugs this session were caught that way.

## Traps already hit

Recorded so they aren't hit twice.

**Measuring**

- **A wrong gold answer measures nothing.** One was shipped ($2,450 vs the
  correct $1,750) by double-counting a base rate. Verify arithmetic with a
  script — `scripts/build-eval-fixtures.py` now computes them.
- **A case with more than one right answer has none.** `pick-search` asked
  "which file mentions Wasita" when three do, and asserted one of them. It only
  surfaced when a model named the other two. Grep the fixture before writing
  the gold.
- **A grading ceiling can measure the ceiling.** Two ambiguity cases had turn
  budgets tight enough that the model got cut off mid-exploration, so the case
  graded budget exhaustion rather than judgement. Discovering that a request is
  under-specified takes reading; leave room for it.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right.
  `eval::normalize` handles it — extend that, don't work around it.
- **…and again, unboundedly.** "There is no `budget.csv`" matched none of ten
  hand-listed negation phrasings. The negation phrasing space has no bottom —
  that case is judge-only now. Reach for `expect.judge` when you catch yourself
  enumerating synonyms.
- **A judge needs room to think before it answers.** At `max_tokens: 512` the
  judge spent the entire budget on reasoning and returned empty content with
  `finish_reason: length`. It is 4096 now, and an unparseable verdict reports
  the stop reason rather than just the empty string.

**Providers**

- **Never believe `finish_reason`.** llama-server reports `stop` alongside
  `tool_calls`. The loop believed it, dropped the calls, ended the run and
  returned an empty string — which graded as a model failure and was a harness
  failure. Any turn containing tool_use blocks is now a tool turn regardless.
  Assume the same class of bug exists for other local servers.
- **A request with `tool_result`s and no `tools` gets an empty completion** from
  llama-server. This is why the compaction summariser is sent flattened prose
  rather than a structured replay.

**Security**

- **A turn boundary is not a security boundary.** Taint was per-run, so every
  guard keyed on it silently reset between chat turns while the content it was
  guarding against stayed in context. Anything scoped to "a run" is worth
  re-checking against "a conversation".
- **`Command::envs()` does not replace the environment, it adds to it.** Every
  MCP server was inheriting mecha's full environment, provider keys included,
  and nothing about the call site looked wrong. `env_clear()` first.
- **A sandbox that silently degrades is worse than none.** The first version
  fell back to unconfined execution when the backend was missing. Since `shell`
  declares narrower capabilities when confined, that would have had the
  interlock trusting a claim nothing was enforcing. It refuses to start now.
- **Wrapping our own errors as untrusted content** made the model invent
  explanations for its own harness's behaviour. Provenance, not capability.

**Environment**

- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
- **`hf download repo --include X Y`** silently ignores `--include` when
  positional filenames are given. Pass filenames positionally *or* use
  `--include`, not both.
- **A pty with no window size renders every TUI frame into 0×0.** Add `stty
  rows N cols M` inside `script`, or the app looks broken when it is fine.
- Free-tier claims in comparison articles are often stale. Exa's own page says
  $10/month recurring credits (~1,400 searches), not the 20,000 some
  aggregators report.
