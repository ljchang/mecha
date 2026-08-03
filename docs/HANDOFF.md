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
| Sandbox | `shell` and MCP servers confined via bubblewrap or docker; no network by default |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Interfaces | `run`, `chat`, `tui` (live input line, steer while it works), `batch` |
| Context | Compaction with tool-call-safe cut points, taint preserved |
| Sessions | Append-only JSONL, resume |
| Eval | 34 cases, 14 tags, scorecard, `--compare`, sandboxes, LLM judge |

71 tests. `cargo clippy --all-targets` is clean and should stay that way.

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

On the original 25 grounded cases, all four models score 23–24/25 with zero
malformed arguments and zero invented tools. **That set saturated** — it is a
floor test, not a ranking test, and it stays in the file as exactly that.

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

## Next features

The three items that gated everything else — measurement, interruptibility,
sandboxing — are done and kept below for their design notes rather than as a
checklist. What is left is genuinely independent; pick by what you want to use.

### ~~Harder eval cases~~ — done

Kept here because the *design* is what matters, not the checkbox.

`RunContext` now carries the three things that are per-run rather than
per-agent: the path jail, the approver, and the budget. `Agent::run_in` takes
one; `Agent::run` uses the agent's own. One agent — one provider connection,
one cached prefix — therefore serves concurrent runs jailed to different
directories under different permissions. Subagents inherit the *caller's*
workspace rather than the one that existed when they were built, which also
closed a real hole: a parent in a sandbox used to delegate to a child still
pointed at the original directory.

On top of that: `"sandbox": true` stages a private fixture copy per case and
allows writes; `"max_turns": N` gives a case its own turn budget;
`expect.verify` runs a command in the case's workspace afterwards and grades
the exit code; `expect.judge` grades a rubric with a second model
(`--judge-provider`, `--judge-model`).

Three things worth keeping in mind about that machinery:

- **Grade the artifact, not the claim.** For codegen, `verify` runs the tests.
  The command hashes the test file first, so a model that edits the tests until
  they pass fails.
- **A judge that cannot answer must fail the case, never skip it.** A case
  whose only real assertion silently evaporates is worse than one that fails
  loudly.
- **Fixtures are generated** (`scripts/build-eval-fixtures.py`), which is how
  the gold answers get computed rather than guessed, and how the katas are
  checked to fail-as-shipped *and* be solvable.

Still open here: the case set is graded per-run, so a flaky judge or a
borderline case shows up as noise. Running each case k times and reporting
pass@k would cost k× and be worth it for the `ambiguity` tag.

### ~~Interruptibility~~ — done

`RunContext` gained two optional fields, both `None` by default so nothing
changes for a caller that doesn't want them.

**`cancel: Option<CancellationToken>`.** Cancelling stops the run at the next
safe point and keeps what it has: `StopCause::Interrupted`, the partial text in
`RunOutcome.text` with a note saying it is incomplete, and the partial assistant
turn appended to `messages` so the session can be resumed from where it was cut.

Three decisions worth not re-litigating:

- **Cancellation is a dropped future.** `Agent::complete` selects between the
  provider call and the token; losing the race drops the provider future, which
  aborts the in-flight HTTP request. There is nothing else to abort.
- **A cancellable run streams**, even when nobody is watching, because that is
  the only way to have the half-written answer when the request is dropped.
  This is why the field is opt-in: a batch worker nobody can interrupt should
  not silently change transport.
- **Tools run to completion.** The cancellation points are the turn boundary
  and mid-stream, not mid-tool. Interrupting a `shell` call half way through a
  write is worse than waiting for it.

Verified against llama-server: interrupting a count-to-400 cut it off at 246
with the partial answer kept in both the terminal and the transcript.

**`queued_input: Option<Arc<Mutex<VecDeque<String>>>>` — steering.** Text the
user types mid-run is drained at the top of each turn and folded into the
message that already carries the tool results, so the model sees "here is what
your tools returned, and also: actually, focus on X" as one user turn and keeps
working. The run is never stopped and restarted. That placement is forced and
also correct: between an assistant's `tool_use` and its results there is no
legal place for a user message, so the results message is the first opening, and
taking it is the difference between steering a run and queueing until it ends.
Both encoders preserve it (Anthropic as a second content block, OpenAI as a
trailing `role: "user"` message after the `role: "tool"` ones).

`mecha tui` is the front-end for it — see below. `mecha chat` deliberately has
no steering: a readline REPL cannot read stdin while a run streams without a
second reader on the same descriptor, and whichever one is blocked when the run
ends steals the user's next prompt line.

Still open: an interrupted run reports **zero usage**, because token counts
arrive in the final SSE frame that never comes. The tokens were spent. Either
estimate them or report them as unknown rather than as zero.

### ~~Sandboxing~~ — done

`[sandbox] kind = "bwrap" | "docker" | "none"`. A confined command gets the
workspace, a read-only system, no home, no environment beyond a named
allowlist, and by default no network. Default is still `none`, because turning
it on cannot be right for every machine — but `mecha tools` says so out loud.

Verified end-to-end through the agent, docker backend: uid 1000, `~/.ssh` absent,
container hostname, 6 environment variables, DNS dead, and files written into
the workspace owned by the user rather than root (`--user`, without which the
agent leaves root-owned files you cannot delete).

Three things worth not re-deriving:

- **A broken sandbox stops the run.** `preflight` runs a real command through
  the real backend at startup. Falling back to unconfined execution would be
  worse than never configuring one, because `shell`'s declared capabilities
  narrow when confined and the interlock trusts them.
- **Only `external_send` narrows**; `private_data` stays true. A confined shell
  still reads the workspace, and `fs_read` is private for exactly those bytes.
  I had this wrong first: narrowing it would have made `shell: cat secrets` set
  no taint where `fs_read: secrets` does, so the cheapest way around the
  interlock would have been the more dangerous tool. There is a test named
  after that hole.
- **`bwrap` does not work on this machine.** Installed, `unprivileged_userns_clone=1`,
  and still fails with `setting up uid map: Permission denied`, because Ubuntu
  23.10+ added `kernel.apparmor_restrict_unprivileged_userns=1` and ships no
  AppArmor profile for bwrap. The docker backend exists because of this. The
  error message says all of that when it fires.

**MCP servers are covered too**, and were the bigger hole: third-party code, not
commands a model asked for out loud.

- `env_passthrough` replaced inheritance. A nosy test server went from 64
  variables including two API keys to 3 and none. This is a **breaking change**
  for any server that relied on inheriting a token — name it in
  `env_passthrough` or set it in `env`.
- `sandbox = true` per server confines it with the global backend, and
  per-server `network` overrides the global switch, because otherwise reaching
  one server's API would mean giving `shell` the network. Verified: a confined
  server reports a container hostname, no `~/.ssh`, no secrets.
- Asking for confinement with no backend set is a **startup error**, not a
  warning. `mecha tools` lists every server and says which are unconfined.

**Taint is per-conversation, not per-run.** It was created fresh inside `run`,
so a chat turn reset it and the lethal trifecta was defeated by pressing Enter.
It now lives on `agent::Conversation` with the messages, and is written to the
session file — provenance cannot be recovered by reading a transcript back, so
without that record, resuming laundered it too. Verified across a process
restart: a page fetched in one session, a file read in the resumed one, and the
outbound call refused.

The type is the fix. A caller that keeps the history keeps the taint; one that
starts a new `Conversation` gets a clean one, which is why batch items,
subagents and eval cases do not contaminate each other. There is a regression
test, and it was checked to fail against the old behaviour rather than merely
pass against the new.

Still open on this axis:

- **A confined MCP server sees the workspace**, so a filesystem server confined
  this way is confined against your home directory, not against your project.
  That is the right trade for most servers and worth knowing.
- **HTTP/SSE MCP transports** are not implemented, and when they are, none of
  this applies to them — there is no child process to confine, and the trust
  question moves to the endpoint.
- **Subagent workspaces.** A subagent inherits the caller's workspace; there is
  no way to give a child a *narrower* jail than its parent, which is the
  natural next capability restriction.

### 1. The remaining surfaces

Roughly independent of each other.

**~~TUI (ratatui)~~ — done, `mecha tui`.** One event loop owns the terminal for
the whole session and the agent runs in a task beside it, so the input line
stays live: Enter starts a run when idle and *steers* one that is already
going. Ctrl-C cancels the run rather than killing the process; Ctrl-C again at
an idle prompt quits. Approval is a modal, because the terminal approver's
`read_line` would fight the event loop for stdin and its prompt would tear the
frame — `setup::prepare_with_approver` exists for that, and only swaps the
approver in `Ask` mode.

Verified by driving it under a pty (`script` + `stty`, since a pty with no
window size renders every frame into a 0x0 area and looks broken). The steering
case that matters:

```
● shell  sleep 6 && echo one
● shell  sleep 6 && echo two
● shell  sleep 6 && echo three
↳ change of plan: skip the rest and just reply with the single word PIVOT  (steering)
PIVOT
```

The fourth command never ran, and the run was never stopped and restarted.

Not done: the `todo` list is not rendered as a live pane, and there is no
tool-call *tree* — nested subagent calls appear flat.

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

### ~~Context compaction~~ — done

`[agent] compact_at_tokens` / `--compact-at`, off by default. Verified against
llama-server on the 16-link audit chain with a 1200-token threshold: it
compacted four times and still answered 16 entries / 847, matching the gold. The
summaries carried the running total forward, which is the part that could have
destroyed the task silently.

The one real bug came from running it, not from the unit tests: replaying the
structured transcript to the summariser means sending `tool_result`s on a
request that declares no tools, and llama-server returns an empty completion.
The summariser now gets flattened prose. Assume the same is true of any
provider — the structured replay was never worth the fidelity.

Still open: the threshold is an absolute token count because nothing here knows
any model's context window. A `context_window` on `ProviderConfig` would let it
be a fraction, and would want the same treatment as pricing — configured, never
guessed.

### 2. Smaller, high-value items

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
  script — `scripts/build-eval-fixtures.py` now computes them.
- **A grading ceiling can measure the ceiling.** Two ambiguity cases were given
  turn budgets tight enough that the model got cut off mid-exploration, so the
  case graded budget exhaustion rather than judgement. Discovering that a
  request is under-specified takes reading; leave room for it.
- **Substring grading measures formatting, again.** "There is no `budget.csv`"
  matched none of ten hand-listed negation phrasings, all of which were mine.
  The negation phrasing space is unbounded — that case is judge-only now. Reach
  for `expect.judge` when you catch yourself enumerating synonyms.
- **A judge needs room to think before it answers.** With `max_tokens: 512`
  the judge model spent the entire budget on reasoning and returned empty
  content with `finish_reason: length`. It is 4096 now, and an unparseable
  verdict reports the stop reason rather than just the empty string.
- **`shell` is enabled inside sandboxed eval cases**, because a codegen case has
  to run its tests. The staged workspace is a copy, so the *fixture* is safe,
  but the command still runs as you — the case file is trusted input. This is
  the same gap that gates feature 2.
- **A case with more than one right answer has none.** `pick-search` asked
  "which file mentions Wasita" when three do, and asserted one of them. It only
  surfaced when a model named the other two. Grep the fixture before writing
  the gold.
- **Never believe `finish_reason`.** llama-server reports `stop` alongside
  `tool_calls`. The loop believed it, dropped the calls, ended the run and
  returned an empty string — which graded as a model failure and was a harness
  failure. The loop now treats any turn containing tool_use blocks as a tool
  turn, and a completed run with nothing to say says so rather than returning
  "". Assume the same class of bug exists for other local servers.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right.
  `eval::normalize` handles it now — extend that, don't work around it.
- **Wrapping our own errors as untrusted content** made the model invent
  explanations for its own harness's behaviour. Provenance, not capability.
- **A turn boundary is not a security boundary.** Taint was per-run, so every
  guard keyed on it silently reset between chat turns while the content it was
  guarding against stayed in context. Anything scoped to "a run" is worth
  re-checking against "a conversation".
- **`Command::envs()` does not replace the environment, it adds to it.** Every
  MCP server was inheriting mecha's full environment, provider keys included,
  and nothing about the call site looked wrong. `env_clear()` first.
- **A sandbox that silently degrades is worse than none.** The first version
  fell back to running unconfined when the backend was missing. Since `shell`
  declares narrower capabilities when confined, that combination would have had
  the interlock trusting a claim nothing was enforcing. It now refuses to start.
- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
- **`hf download repo --include X Y`** silently ignores `--include` when
  positional filenames are given. Pass filenames positionally *or* use
  `--include`, not both.
- Free-tier claims in comparison articles are often stale. Exa's own page says
  $10/month recurring credits (~1,400 searches), not the 20,000 some
  aggregators report.
