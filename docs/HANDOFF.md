# mecha — handoff

Where the project stands and what is actually left to do. Written to be picked
up cold.

Two companion documents, so this one can stay short:

- [`CLAUDE.md`](../CLAUDE.md) — why each subsystem is shaped the way it is. The
  canonical design document. This file deliberately does not restate it.
- [`HISTORY.md`](HISTORY.md) — what was built and when, and the traps hit along
  the way. Everything here that turned into "done" moved there.

**Keeping this file honest is a procedure, not a habit.** Run the `handoff`
skill (`.claude/skills/handoff/`) at the end of a session that changed
behaviour: it walks every open item below, verifies against source whether it
shipped, moves the ones that did into `HISTORY.md`, and re-checks the counts
and machine facts that rot silently. [`README.md`](README.md) in this directory
maps which document holds what.

---

## Where the work is

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.0**.
CI runs build, test, clippy and rustfmt on every push and pull request; the
documentation site builds from `website/` and deploys to
<https://ljchang.github.io/mecha/>.

Every commit was verified to build and pass tests **in isolation**, so the
history bisects rather than merely ending in a good state.

First thing to run in a fresh context:

```bash
cargo test --workspace && cargo clippy --all-targets --all-features
```

Expect **468 tests**, no warnings — verified 2026-08-05:

| Suite | Count |
|---|---:|
| `mecha-core` unit | 315 |
| `mecha-cli` unit | 73 |
| `mecha-mail` unit | 66 |
| integration (`mcp_server` 6 + `sandbox_backends` 7) | 13 |
| doctest | 1 |

The integration tests need docker (with `debian:stable-slim` and `python:3-slim`
pulled) and `python3`; without them they skip and say so. CI sets
`MECHA_TEST_REQUIRE_BACKENDS=1` so a missing backend fails instead of quietly
passing — a silently skipped test reads exactly like a passing one.

**The system prompt is wired.** `~/.mecha/config.toml` points
`[agent] system_prompt_file` at `prompts/agent.md`. It was not, for the whole
life of the project before 2026-08-03 — `RunConfig` recorded
`system_prompt: null`, and the model had none of that file's guidance. Check it
is still set before concluding anything about model behaviour.

## What exists

A working agent harness, used and measured rather than just compiled.

| Area | State |
|---|---|
| Providers | Anthropic (raw HTTP, **verified live**) + OpenAI-compatible (llama-server, vLLM, Ollama) |
| Agent loop | Streaming, tool dispatch, parallel execution, forced final answer |
| Tools | `fs_read/write/edit/list`, `shell`, `http_fetch`, `todo`, `web_search`, `ask_user` |
| Planning | `Phase::Plan` hides writing tools structurally — not offered, not dispatchable |
| MCP | stdio client; per-server on/off; capability overrides that only widen |
| Memory | pkg wired as an MCP server — the user's mail, Slack, calendar, conversations |
| Subagents | `Agent` wrapped as a `Tool`, allowlisted registry, per-profile model |
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — with fall-through |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Sandbox | `shell` and MCP servers confined via bubblewrap or docker; no network by default |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Context | Two-pass compaction: thin tool results, then summarise. Taint preserved |
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval` |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit/send/reject; edits mined as writing reflections |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, systemd unit in `scripts/` |
| Learning | the full arc: reflect-on-close → nightly rumination → counterfactual validation (steers/denials trace-graded) → gated proposals (`mecha proposals`); git-backed store under `~/.mecha/learning`; rules carry id/sources/created_at, validate feeds a per-rule outcome ledger with regression bisection, and `mecha rules` retires through the same gate (`eval --ab-rules` for the coarse A/B) |
| Eval | 36 cases, 15 tags, scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `pkg-cases.jsonl` — 8 memory/interlock cases against fixture MCP servers (`--mcp-file`) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Verified 2026-08-05:**

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, `total_slots=1` — MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`) |
| 8081 | gemma-4-E4B | down; nothing currently depends on it |
| 8082 | gemma-4-26B-A4B | up, `total_slots=1` — the eval judge and nightly validate's judge |
| 8888 | SearXNG | up (docker, JSON format enabled) |

**`-np 1` is load-bearing.** The llama-server build in use defaults to **4
parallel slots**, which silently splits `-c` across them: for a period on
2026-08-04 every request ran against **8192 tokens of context, not 32768**,
while mecha's `context_window` said otherwise. Past 8192 the server
context-shifts instead of erroring, so the model saw a mangled transcript and
returned *empty completions* — the mysterious empty-EndTurn deaths in the k=5
compaction runs were this, not a mecha regression, and every scorecard taken
between the two restarts is confounded. Check
`curl :8080/props | jq .total_slots` is 1 before believing any measurement.

Start scripts are in `scripts/` (`start-moe-mtp.sh`, `start-e4b.sh`,
`start-gemma26.sh`); they resolve the model through `$HOME` with `HF_HUB` and
`LLAMA_SERVER` overrides. Config is `~/.mecha/config.toml` (providers `local`,
`small`, `gemma26`, `anthropic`).

### Standing machinery on this machine

- **Reflect-on-close**: `~/.mecha/config.toml` carries a `session_end` hook
  running `nohup mecha reflect -p local ... &` — every recorded session is mined
  minutes after it closes.
- **Nightly rumination**: `mecha-ruminate.timer` (systemd user, 03:30,
  `Persistent=true`, linger on) runs `scripts/ruminate.sh`: reflect → distill →
  validate `--unprocessed-only` (judge: gemma26) → learn
  `--holdout 0.25 --propose` → `rules propose-retirements`. Logs land in
  `~/.mecha/learning/logs/<date>.log`; pending proposals wait in
  `mecha proposals`. **Confirmed enabled 2026-08-05.**
- **Triggers are built but nothing schedules them here.** `mecha trigger daemon`
  is not installed — `scripts/mecha-triggers.service` is written and untried
  (confirmed absent from `systemctl --user` 2026-08-05). Installing it is three
  lines, and until then triggers fire only when someone runs
  `mecha trigger tick` or `run` by hand.
- **Both consume `~/.cargo/bin/mecha`**, not the repo build — reinstall
  (`cp target/release/mecha ~/.cargo/bin/`) after changing anything in the
  learning path, or the automation runs stale behaviour.
- The learning store (`~/.mecha/learning`) holds **zero live rules** — the one
  early rule was reverted with its poisoned reflection — so everything from here
  accumulates from real usage through the gate.

### The Anthropic key

`ANTHROPIC_API_KEY` is set in `~/.bashrc`. Two gotchas, both of which cost time:

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard near the top). The export sits well below it, so a non-interactive shell
  — which is what tooling runs — never reaches it. Load it explicitly:
  `eval "$(grep '^export ANTHROPIC_API_KEY' ~/.bashrc | tail -1)"`.
- **Take the *last* match, not the first.** There were two exports for a while,
  a placeholder above the real key; `grep -m1` silently found the placeholder
  and produced a `401 invalid x-api-key` that looked like a bad key.

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
   grammar-constrains tool calls; that is why malformed-argument counts are zero
   across the board. Don't conclude anything about a model's tool reliability
   from an unconstrained sampler.

The cases added since (`long-horizon`, `codegen`, `synthesis`, `ambiguity`) do
discriminate. qwen3.6-35b-a3b judged by gemma-4-26b-a4b scored **32/34** on the
set as it stood then (`results/qwen-hard-v2.json`):

- **long-horizon 2/2**, at ~17.5 turns — it walks a 16-link chain without losing
  the running total, and does not take the shortcut of summing the decoys.
  Confirmed at n=5 on 2026-08-03: `chain-total` is **5/5** uncompacted,
  `chain-largest` **4/5**. A single earlier failure looked like a regression and
  was variance — which is the whole argument for pass^k.
- **codegen 2/2** — implements `median`, finds the one-line duration-parsing
  bug, and runs the tests itself. Graded by running them, not by asking.
- **synthesis 2/2** — finds the majority figure and the outlier, and notices
  which report supersedes which.
- **ambiguity 8/9 across the tag** once `ask_user` existed *and* the cases were
  rewritten to grade the trace. The measurement history is the lesson: a clean
  A/B said the tool made no difference (6/9 either way) and the transcripts said
  otherwise — without it the model burned **30 tool calls** and died on the turn
  ceiling with a correct answer; with it, it asked in **3** and failed a rubric
  that demanded it ask for two missing things at once. A large real improvement
  was invisible to the grader. `ambiguous-rate` now asserts `tools: ["ask_user"]`
  and `false-premise` asserts `forbid_tools: ["ask_user"]` — because the right
  move there is *not* to ask, the file simply does not exist. Read the
  transcripts before believing a score.

Only `ambiguity` and `synthesis` have a judge in the loop, and judges disagree
with themselves across runs. Read the answer before believing a single verdict.

**Scorecards in `results/` taken before the fixture expansion are not comparable
to ones after it.** The new fixtures took the shared workspace from 11 files to
44, so every case that searches the whole workspace got harder — two of them
started failing on turn ceilings calibrated against the smaller tree. If you add
fixtures, expect to recalibrate, and re-baseline every model rather than
comparing across the boundary.

The compaction arc — seven measured arms, and the finding that a summariser
preserves *what is true* while dropping *how far you got* — is in
[`HISTORY.md`](HISTORY.md). It is the reason compaction is shaped the way it is,
and it is worth reading before changing that code.

---

## What to do next

Every item below was verified against source on 2026-08-05 to still be unbuilt.
Ordered by value per unit of effort, not by size.

### Cheap, and worth doing first

- **Write a trigger's report into its own workspace.** A trigger already carries
  an optional workspace (`mecha-core/src/trigger.rs`), but nothing writes a
  report file into it — the run's answer exists only in the session transcript,
  which is why the TUI reads it back from there. An unattended run that leaves a
  durable artifact is the difference between a scheduled agent and a scheduled
  agent you can actually use. This is an afternoon.
- **Batch review in the outbox.** `mecha outbox send` takes exactly one id
  (`mecha-cli/src/commands/outbox.rs`), as do `show`/`edit`/`reject`. An
  overnight triage trigger that stages nine replies is then nine invocations.
- **Re-inject the todo list at compaction time.** `compact.rs` has no knowledge
  of the todo tool, so the model sees its list only through the echo in the last
  `todo` result — a compaction that summarises past that echo loses it. The list
  is exactly the "how far you got" state the summariser is measured to drop.
- **`calendar_freebusy` on the unified mail surface.** Nothing in
  `mecha-mail/src` implements it, and every scheduling question needs it.
- **Re-baseline `ambiguity` and `long-horizon` at k=5.** No scorecard in
  `results/` records `runs: 5` outside the compaction arc, and these are the two
  tags whose single-run numbers move.

### Structural gaps

- **MCP resources are not implemented.** `mecha-core/src/mcp.rs` advertises
  `"capabilities": {}` and speaks only `tools/list` and `tools/call`. No
  `resources/list`, no `resources/read`. Whenever this is closed, resource
  content must be marked `.from_outside()` — it is third-party text by
  definition.
- **HTTP/SSE MCP transports.** `McpServerConfig` carries only `command`/`args`,
  so every server is a local process. Worth knowing before building it: none of
  the process confinement — `env_passthrough`, `sandbox`, per-server `network` —
  means anything for a remote endpoint, so the security story has to be
  rewritten rather than inherited.
- **Subagents cannot narrow the jail.** `SubagentProfile` narrows tools,
  `max_turns`, model, provider and `trusted_output`, but has no workspace field;
  `subagent.rs` passes the caller's `ToolCtx` verbatim, so a child's jail is
  always exactly its parent's. A subagent that should only read one directory
  cannot be expressed.
- **Per-command approval policy.** `ModeApprover::approve` takes the tool input
  and ignores it (`mecha-core/src/tool/mod.rs`), branching only on mode and
  `read_only`. So `shell: ls` and `shell: rm -rf` are the same decision. There
  is no per-command rule surface in config to hang this on yet.
- **Structured output has no provider abstraction.** The `Provider` trait exposes
  only `id`/`default_model`/`complete`. GBNF, `guided_json` and
  `output_config.format` are all spellings of the same idea and none is reachable.
- **A Landlock + seccomp sandbox backend.** `Backend` is `None`/`Bwrap`/`Docker`.
  The sandbox research measured Landlock at ~1.28 ms and it works on this box,
  where bwrap does not — so the measured winner is the one that is unimplemented,
  and docker remains the only working option here.
- **In-run verification / a convergence primitive.** Nothing in `agent.rs` tests
  a post-condition; there is no runtime "is it done yet". The research's own
  answer is the starting point: it has to be a command's exit code, not a
  model's opinion. `compact_validate` is the only in-run verifier that exists.
- **Programmatic tool calling** (a `code` tool that calls other tools from inside
  a program). Two hazards to solve first, both named in the research: taint must
  update *within* a running program, and approval for a program that makes
  thirty calls is not thirty approvals.

### The learning system

The arc is complete and running nightly. What is missing is refinement:

- **The sliding window of recent raw reflections never shipped.** Prompt assembly
  chains user rules then consolidated rules; the third leg — a window of recent
  unconsolidated reflections — was designed and not built.
- **Rules are scoped by domain, not by tool.** `Rule` has no `scope` field, and
  nothing injects rules into a tool's own block.
- **Rules that are facts should graduate to pkg.** No classifier routes
  fact-shaped rules into `kg_upsert` as staged candidates; `distill.rs` pushes
  episodes only.
- **The positive signal is unread.** Reflection mines only edited-then-sent
  outbox items. An item sent *unedited* is evidence that whatever produced it
  was right, and nothing looks at it.
- **LEAP-in-production.** Rumination mines interventions only. Learning from
  graded eval cases — sampling known-outcome examples rather than waiting for a
  correction — was ported in design but not in code.
- **No correction-rate-over-time query.** `mecha sessions stats` counts sessions
  and `mecha rules list` gives per-rule tallies; neither answers "are
  interventions per session going down", which is the only number that says the
  system is working.
- **The CIPHER tier** — per-context preferences, embedded and retrieved top-k —
  exists as a comment and nothing else.
- **A `/learning` TUI view.** The store is files by design so it can be read
  without tooling, but nothing surfaces it in the interface.

One item needs a human who remembers the intent: the **edit-distance gate** is
described as observed working live, but no threshold exists in code — the
behaviour came from the reflector model declining. If the design was always
"the model declines", the item is obsolete rather than open.

### Triggers

- **A trigger's path jail defaults to `$HOME`, and `$HOME` contains
  `~/.mecha/`.** `Trigger::workspace` is `Option<PathBuf>`; unset, it falls
  through `setup::prepare_tools` to `std::env::current_dir()`, and
  `scripts/mecha-triggers.service` sets `WorkingDirectory=%h`. A trigger with
  filesystem tools and no explicit workspace can therefore `fs_read` the mail
  OAuth tokens, the session transcripts and the learning store. The shipped
  `morning.toml` escapes only because its tool allowlist is `mail__*` — the
  jail is not what saves it. **Not trigger-specific**: `mecha chat` started
  from your home directory has the same shape. The interlock remains a
  backstop, but a jail rooted where the secrets live is the
  silently-degrading-sandbox pattern. Two fixes: default the workspace to
  `~/.mecha/workspaces/<name>/` at `add` time (which also gives triggers
  stable cross-run read-back, see `PUBLIC-SURFACE-DESIGN.md` §2.2c), and
  refuse in `setup` any workspace that contains the mecha home.
- **File watchers.** `Trigger` has no watcher kind. Needs debounce and a
  "what changed" payload injected into the prompt.
- **Inbound webhooks.** Nothing listens. The interesting part is that the payload
  must arrive marked untrusted — it would be the first time the interlock's rules
  applied to a *prompt* rather than to a tool result.

### TUI polish

- **Steering and queuing are the same key.** Enter starts a run when idle and
  steers one already going; there is no way to queue a follow-up instead.
- **No `/export` or copy.** `NAMES` lists eleven commands and none of them get
  the transcript out.
- **`NO_COLOR` is honoured only by the plain CLI renderer.** The TUI hardcodes
  colours inline; there is no semantic colour table.
- **No keymap configuration.**

### Larger, and deliberately not started

- **The public surface.** `publish`/`read`/`write`/`drain` verbs, a
  request-type manifest generating schema/form/validators/tool declarations,
  published bundles (reports, dashboards, marimo notebooks), origin isolation
  and CSP content classes. Nothing is built, on purpose. Two documents:
  [`PUBLIC-SURFACE-RESEARCH.md`](PUBLIC-SURFACE-RESEARCH.md) is the survey and
  the argument, [`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) is what
  the user decided to build — **read the design doc for what, the research doc
  for why.** Note this is unrelated to the repository being public; it is a
  surface for the user's correspondents.

  The decisions in one paragraph: our own server (`mecha-surface`, one Rust
  binary, SQLite/WAL, on a VPS, forbidden from depending on `mecha-core`),
  reached over two scoped **API keys** rather than OAuth, push–pull posture
  unchanged; **immutable content-addressed bundle versions behind a moving
  alias**, mirrored to `~/.mecha/bundles/` so an agent can read what it
  published; **templates as the extension point**; **marimo first-class** on
  its own origin with `wasm-unsafe-eval` + COOP/COEP, vendored assets, and a
  publish that *fails* on any surviving external reference; a four-layer
  **quarantine** whose real work is a tool-less extraction pass turning a
  stranger's prose into typed fields before anything privileged sees it, with
  the injection classifier as a label and never a gate; **scheduling** as
  book-me first, then group availability *seeded by mine* so participants
  never see a blank grid; WebMCP designed for and deferred behind a flag;
  compliance deferred deliberately, with the manifest fields kept.

  The cheapest prerequisite is unchanged and still unstarted: mining twelve
  months of mail for the request types that actually recur, which no artifact
  records yet — and it should happen before any field list is frozen.
- **Slack as a transport.** Zero lines exist. The blocking decision is the
  identity model, not the socket.
- **Public benchmarks.** The Terminal-Bench adapter (`bench/`) is written and
  smoke-tested at `n_tasks: 1`; the oracle arm64 sweep is incomplete and the full
  89-task run has never been made. AgentDojo (for the interlock) and a SWE-bench
  Bash Only control are named in the research and unstarted.
- **`mecha replay --json` is not wired into CI.** `scripts/replay-regression.sh`
  consumes it locally, which is the standing regression check; making it a
  workflow needs a single-slot llama-server that CI does not have.

---

## Accepted limitations

These are not to-do items. Each is a deliberate trade, recorded so nobody
"fixes" one without knowing what it costs.

- **`shell` is not treated as an untrusted source.** Taint cannot see inside a
  command, so a command that fetches a hostile page does not arm the interlock
  the way `http_fetch` does. The mitigation is the sandbox, and it is why
  `[sandbox] kind` matters.
- **A confined MCP server sees the workspace.** It is the one writable bind on
  both backends. Right for most servers, wrong for some, and worth knowing which
  you are running.
- **`shell` is enabled inside sandboxed eval cases**, with approval forced open.
  The fixture is safe because each case gets a private staged copy, but the
  command still runs as you unless `[sandbox]` is configured — and its default
  is `none`.
- **A failed trigger run is not retried.** The next slot is the retry. Adding
  retry means deciding what a half-completed unattended run owes you.
- **Gmail drafts, a local mail cache, and past-correspondence context** were all
  considered and deferred, not forgotten.
- **pkg gets no self-scoped read tool.** Capability overrides widen only, so a
  memory read arms `untrusted_input`. Revisit only if self-retrieval taint proves
  to hurt in practice.
- **Retrieval at turn start is rejected**, because it arms the trifecta at turn
  zero for every run whether or not memory was needed.
- **Rule retirement has no decay, TTL, or usage-based eviction**, and no policy
  built on model-rated confidence. Only measured harm argues for retirement, and
  a human accepts the argument.
