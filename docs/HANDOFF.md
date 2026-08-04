# mecha — handoff

State of the project and what to build next. Written to be picked up cold.

---

## Where the work is

**All of it is on `main`**, and `main` is level with `origin/main`. The working
tree is clean.

Every commit was verified to build and pass tests **in isolation**, so the
history bisects rather than merely ending in a good state.

First thing to run in a fresh context:

```bash
cargo test && cargo clippy --all-targets -- -D warnings
```

Expect **149 core + 20 CLI unit tests, 13 integration tests, 1 doctest** — 183,
no warnings. The integration tests need docker (with `debian:stable-slim` and
`python:3-slim` local) and `python3`; without them they skip and say so. In CI,
set `MECHA_TEST_REQUIRE_BACKENDS=1` so a missing backend fails instead of
quietly passing.

**The system prompt is wired now.** `~/.mecha/config.toml` points
`[agent] system_prompt_file` at `prompts/agent.md`. It was not, for the whole
life of the project before this — `RunConfig` recorded `system_prompt: null`,
and the model had none of that file's guidance. Check it is still set before
concluding anything about model behaviour.

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
| Replay | `replay.rs` extracts and diffs trajectories (pure half only — no driver yet) |
| Eval | 36 cases, 17 tags, scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks |

`cargo clippy --all-targets` is clean and should stay that way.

### What the tests actually cover

Six of this project's load-bearing claims used to be backed by a paragraph in
this file describing a run that happened once, by hand, in a session that was
over. They now re-run:

| Claim | Where |
|---|---|
| A transcript round-trips its taint, and taint records *merge* on load | `session.rs` |
| The MCP child environment is an allowlist, not an inheritance | `mcp.rs` + `tests/mcp_server.rs` |
| The Anthropic body never sends what 400s, and the cache breakpoint is placed right | `provider/anthropic.rs` |
| Fragmented tool-call arguments reassemble; calls survive a mislabelled `finish_reason` | `provider/openai.rs` |
| A broken sandbox fails preflight instead of degrading to unconfined | `tests/sandbox_backends.rs` |
| A confined command/server loses the network, your home, and your environment | both integration files |

The split that matters: **unit tests for your own code, integration tests for
what needs real execution, eval cases for what needs a real model.** A
`ScriptedProvider` replays what you *believe* a provider does, so it cannot
catch a provider violating that belief — which is where every expensive
provider bug here came from. Conversely an eval case cannot tell you *which*
layer broke, which is how a dropped-tool-call harness bug once graded as a model
failure.

Still uncovered: the whole `mecha-cli` crate, and bwrap's actual confinement
(it fails on this machine, so that test asserts the quality of the error message
rather than the happy path).

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

[redacted: operational detail — see docs/OPERATIONS.md]
time:

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard around line 5). The export is well below it, so a non-interactive shell
  — which is what tooling runs — never reaches it. Load it explicitly:
[redacted: operational detail — see docs/OPERATIONS.md]
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
  decoys. Confirmed at n=5 on 2026-08-03: `chain-total` is **5/5** uncompacted,
  `chain-largest` **4/5**. A single failure seen before this was measured looked
  like a regression and was variance — which is the whole argument for pass@k.
- **codegen 2/2** — implements `median`, finds the one-line duration-parsing
  bug, and runs the tests itself. Graded by running them, not by asking.
- **synthesis 2/2** — finds the majority figure and the outlier, and notices
  which report supersedes which.
- **ambiguity 2/3 → 8/9 across the tag** once `ask_user` existed *and* the cases
  were rewritten to grade the trace. The measurement history is the lesson: the
  clean A/B said the tool made no difference (6/9 either way), and the
  transcripts said otherwise — without it the model burned **30 tool calls** and
  died on the turn ceiling with a correct answer; with it, it asked in **3** and
  failed a rubric that demanded it ask for two missing things at once. A large
  real improvement was invisible to the grader. `ambiguous-rate` now asserts
  `tools: ["ask_user"]` and `false-premise` asserts `forbid_tools: ["ask_user"]`
  — because the right move there is *not* to ask, the file simply does not
  exist. Read the transcripts before believing a score.
- **the old ambiguity note, kept for context: 1/3** — the weak spot, and the one that moves between runs
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

Ordered by what I would actually do first, not by size.

### 1. ~~Tell the model to keep a todo list~~ — done 2026-08-04, and the answer is no

The paragraph is in `prompts/agent.md` (imperative: "your first tool call is
`todo`") and `todo`'s description now says to call it first for anything over
three tool calls. qwen3.6-35b-a3b called `todo` **zero times in 20 eval
case-runs** either way, and `chain-total-compacted` stayed **4/5 in both arms**
(thinning-only baseline: 4/5, control 5/5). Three probes localised why, and
they also say the gap this targeted is already closed:

- The model keeps a list *flawlessly* when the **user turn** asks for one —
  updates every step, batches `todo` with the action in the same turn — and
  ignores the identical directive in the system prompt. Delivery was verified
  in the recorded `RunConfig` both times: an instruction-following gap in the
  model, not a wiring bug.
- Moving the directive into the tool description got adoption once, on
  genuinely sequential work — as a single static item it never updated. A
  checkmark, not a position ledger.
- Across all **15 compacted chain runs** taken 2026-08-03/04, **no failure was
  a position failure**: every walk read exactly 16 entries, no restarts, no
  early END. Thinning already fixed the mode todo was meant to fix. The three
  misses were all in the *total* (877 and 717 for 847) with position and count
  right — once by summing its own **correct** 16-row table wrong. The residual
  mode is value accumulation, which a *running total kept in the list* would
  address, and which this model will not maintain from prompting alone.

Also learned along the way: the model has **no read path** to the `Mutex` — it
sees the list only through the echo in its most recent `todo` result. Survival
through compaction therefore depends on updating often enough that a fresh echo
sits in the un-summarised tail. If this is ever revisited, the machinery worth
considering is re-injecting the list at compaction time, not more prompting.

Both changes are kept — a stronger model may well follow them, and they cost
nothing here. But **the `todo` description change alters the tool surface of
every eval case**: re-baseline before comparing any full-set scorecard across
this boundary.

### 2. ~~Pin the sampler~~ (done 2026-08-04), then build the replay driver

The sampler half is done, and both of this file's earlier assumptions about it
were wrong in ways that only running it showed:

- **`temperature = 0.0` is unusable on qwen3.6.** Greedy decoding walks into
  verbatim repetition loops that sampling noise would have broken: a limerick
  request spent its entire 4096-token budget repeating one line of its own
  reasoning, where the server-default control answered in 1677 tokens. Pinning
  greedy would have quietly degraded every eval case with long thinking.
- **A seed makes sampling repeatable, but only sequentially.** At
  `temperature = 0.8, seed = 42`, identical requests repeat token-for-token —
  reasoning included — when run one at a time, and *stop* repeating the moment
  another request shares the batch: llama-server's continuous batching perturbs
  the numerics, seed or no seed.

So the pin is `temperature = 0.8` — the server default every prior measurement
already ran at, so no comparability boundary — **plus `seed = 42`**, on all
three local providers. `ProviderConfig` gained both fields; the
OpenAI-compatible provider sends them; the Anthropic provider **refuses either
at startup** (rejecting beats silently dropping — someone who pinned the
sampler must not believe it is pinned where it cannot be); `RunConfig` records
both, so a session file now says whether its run was repeatable. Verified
end-to-end: two `mecha run`s produce byte-identical answers.

What follows from the concurrency caveat: the replay driver must drive
requests **sequentially** (it naturally would — one conversation), and the
eval at `--concurrency 4` stays pass@k-shaped. `--concurrency 1` is now the
deterministic mode, at ~4× the wall clock.

Then the driver — **also done 2026-08-04**. `replay_run.rs` is the impure half
beside the pure `replay.rs`: a registry of tools that answer from the recording
(`replay_registry`), and `drive`, which feeds the recorded user turns to an
agent one at a time and diffs what it did. `mecha replay <session>` rebuilds
the run from the recorded `RunConfig` — its system prompt, tool surface,
budgets — not from today's flags, and takes `--on-divergence=stop|error|live`
(`stop` default; `error` exits nonzero on *any* divergence, for CI; `live`
abandons the recording and continues on real tools under the configured
permission mode). Nine unit tests cover the mechanics, including two that
drive a `ScriptedProvider` through a full faithful and a full divergent run.

Decisions that were not obvious until it ran:

- **A structural divergence kills the recording for every tool**, via a shared
  cursor and the run's cancellation token — the model gets a refusal, the run
  stops at the next safe point, and later recorded turns are never fed.
  Argument-only differences replay the recorded result and keep going; the
  final diff reports them and the caller judges.
- **The spec the model sees is the live tool's**, name/description/schema —
  deliberately, because a changed description is part of what a replay
  measures. A recorded tool missing from today's registry is an error, not a
  silent shrink of the surface. This bit immediately: subagent tools like
  `research` are built by full setup, not `prepare_tools`, so the command uses
  `setup::prepare` and borrows the discarded parent agent's registry.
- **`-p`/`-m` override the recorded provider/model**, which is the payoff:
  cross-model replay on real work. When `-p` is given without `-m`, the model
  falls back to the *new provider's* model, not the recorded name — sending
  qwen's name to gemma's server was the first bug this surfaced.
- **Replayed results carry no `external` marking** — the transcript does not
  record per-result provenance, so a replay's taint can be less armed than the
  recording's. Recorded interlock refusals replay verbatim regardless. Known
  approximation, documented in the module.

Verified end-to-end, all three ways it can go: a recorded qwen session with
tool calls **replays with zero divergence** under the pinned sampler; the same
session replayed on gemma-4-26b-a4b reported exactly one **argument-only**
divergence (`entry-735e.md` for `audit/entry-735e.md` — the handoff's own
example of a spelling, not a behaviour change); and replaying the todo-probe
session on gemma stopped at call #0 with a **structural** divergence and exit
1 under `error` — gemma opens with `todo` where qwen recorded `fs_read`.
That last one is also a finding for item 1: **gemma obeys the todo-first
instruction qwen ignores.** The todo result is model-specific.

Why it was worth doing: this project's case set has **saturated once already**,
and the replacement cost a full session of hand-writing cases, four of whose
graders were wrong before they were right. Replay turns every real session into
a regression case. Still open, now cheap: point it at a longer recorded session
as a standing regression check, and wire `--json` output into something CI can
diff.

### 3. pkg — design settled 2026-08-04, talked through with the user

The open questions here are now decisions. Recorded in full because they were
reached by argument and would be easy to re-litigate cold.

**The three-layer architecture.**

- **Systems of record** (Google Calendar, Gmail, GitHub) hold live truth.
- **mecha is the actor**: it reads and (eventually, via the outbox) writes
  those systems through MCP. "What's on Thursday" is a live-calendar query,
  never a pkg query — pkg would answer with a summarized, possibly stale copy.
  mecha must never "add an event" by writing to pkg: that puts a fact in the
  graph and nothing on the calendar.
- **pkg is the derived layer**: distilled context — who someone is, when you
  last met, what was decided, project state. It learns about the world by
  ingestion, after the fact. One refinement: pkg is the read model for
  anything that has a system of record elsewhere, and the **system of record
  only for what has no other home** — relationships, decisions, episodes,
  tasks-as-the-user-conceives-them. (If a real task tracker is ever adopted,
  tasks migrate to rule one.)

**Retrieval timing: on demand, kept.** The decisive argument: a pkg read arms
both taint legs at once, so session-start retrieval would mean no session
could ever use `web_search`/`http_fetch` after turn zero. On demand pays that
price only when memory is actually needed, only from that moment. Corollary
worth teaching the model: **web before memory** — both orders end equally
tainted, but only one gets the outbound work done first.

**The weak link is recognition, not timing** — measured, n=2 but vivid: asked
"my current projects" with "pull from my knowledge graph", the model answered
beautifully from pkg; asked about "my main research focus" with the source
unnamed, it web-searched a nonsense query, never touched pkg, flailed through
fifteen `fs_list` calls, and died on the context window. The fix is prompt
coverage (projects/goals/tasks/deadlines/"my X" are memory questions) plus
trace-graded eval cases, not a retrieval scheduler.

**One graph, not a separate mecha store.** `kg_upsert` already *requires* a
`source` field (`agent:mecha`) and `kg_search` filters by tags — provenance
scoping is built in, so separation-by-store would duplicate it while
splitting answers like "what are my projects" forever. The user explicitly
wants mecha writing and *curating*: staging facts and connections **in
flight, as tasks teach them**, flagging contradictions and duplicates rather
than silently picking a side. Review-fatigue is handled by policy, not
architecture: the nightly review can bulk-skim `agent:mecha` while reading
personal facts carefully. What does *not* go to pkg: how mecha should behave
(git), raw run history (sessions JSONL), measurements (`results/`). Only what
the user would ask a personal assistant later.

**Caveats recorded with the decision:**

- **Self-retrieval costs taint.** Capabilities are per-server and the override
  only widens, so mecha reading its own notes back arms the interlock like any
  pkg read. Records cannot vouch for themselves through an untrusted channel.
  If it hurts, the fix is pkg-side: a read tool scoped to self-authored +
  curated records. Not needed yet; web-first ordering covers it.
- **`kg_upsert` stages `fact` and `alias` only — no episode kind.** Session-end
  distillation lands as episode-shaped facts until pkg grows one. Queue with
  the pending `readOnlyHint` annotations as one small pkg work-package.
- **Trace-graded pkg eval cases need a fixture MCP server**, because evals are
  deliberately `--mcp`-off and the real pkg is neither deterministic nor
  machine-independent. This converges with the open multi-turn interlock case,
  which needs a fixture tool returning `.from_outside()` content — one small
  fake server can be both.

**Roadmap order that falls out:** prompt widening + write habits (now) →
hooks, with session-end distillation as the first consumer → outbox in core →
calendar *reads* → calendar writes and email through the outbox. Calendar
writes are the trifecta in one tool (invites send; descriptions exfiltrate),
so they do not jump the queue ahead of the outbox.

### 4. Smaller, high-value items

- **Hooks** — pre-tool / post-tool / session lifecycle. Lets policy, redaction,
  and logging attach without touching the loop.
- **Structured-output abstraction** — a `structured_output` knob on `Provider`
  that each backend spells natively (GBNF for llama.cpp, `guided_json` for
  vLLM, `output_config.format` for Anthropic). Don't hardcode GBNF.
- **TUI polish** — the `todo` list is not a live pane, and nested subagent calls
  render flat rather than as a tool-call tree. Both were asked for.
- **Public benchmarks** — tau-bench fits best (it grades tool-call traces, which
  is what this rig grades); SWE-bench Verified next, since the `codegen` cases
  already use its shape. Both free. Worth a sprint once the case set stops
  discriminating.
- **pass@k in the eval** — cases are graded per-run, so a flaky judge or a
  borderline case shows up as noise. Running each case k times would cost k×
  and is worth it for the `ambiguity` tag specifically. Now also worth it for
  `long-horizon`, which moves more than this file used to claim.
- **`context_window` on `ProviderConfig`** — the compaction threshold is an
  absolute token count because nothing here knows any model's window. Would let
  it be a fraction, and wants the same treatment as pricing: configured, never
  guessed.
- **A multi-turn interlock case.** The machinery exists — `"prompt": [...]`,
  `expect.taint`, `expect.blocked_sends` — but nothing in the eval registry is
  an untrusted *source*. `fs_read` is private-but-trusted, so the two
  `injection` cases test the model's resistance, not the interlock. It needs a
  fixture tool that returns local content marked `.from_outside()`, or a case
  that requires the network. **That decision is unmade**, and it is the last
  thing standing between here and grading the trifecta end to end.

### 5. The remaining surfaces

Roughly independent of each other.

**Slack DM.** Socket Mode app in an existing workspace (no new workspace
required). The hard requirement: it must share one session store with the CLI,
or you have two assistants that don't know each other. Decide the identity
model before writing the transport.

**Email / calendar.** Gmail + Graph APIs. **Draft-only, never send** — the
outbox pattern belongs in core as a first-class concept, not as per-tool
politeness. Do not start this before the outbox exists.

Settled with the user 2026-08-04: **mecha is the email actor** — reads,
drafts, and (through the outbox) actions — while pkg mines mail through its
own ingestion to fill the graph, and never becomes the door mecha acts
through. Same three-layer split as calendar (item 3).

And the outbox is more than the safety gate — **it is the first
correction-capture point** for the general self-learning system below: every
staged draft the user edits before sending yields `diff(staged, sent)` as a
contextual correction with the thread and recipient attached — structural
capture, where flowmail needed UI for it. pkg's role in drafting is the
relationship context flowmail's cards provided — who the recipient is, the
register used with them — composed with the learned rules and the live
thread.

**Reflexion — mecha's self-learning, two systems, three stages.** Wanted by
the user explicitly, modeled on flowmail's pair of learning loops
(`~/Github/flowmail/dev_docs/CORRECTION_SYSTEM.md`, Reflexion/LEAP-style).
Two distinct systems, because they learn different things from different
signals:

- **Writing** — learns the user's voice. Signal: the edit. Objective:
  **minimize edit distance between what mecha produces and what the user
  keeps.** Capture is structural: outbox `diff(staged, sent)` for email; for
  file-writing, the diff between what a session wrote and how the user's
  later edits left the file — sessions record the former, which is what makes
  this capturable at all.
- **Behavior** — learns how to perform tasks. Signal: the user stepping in.
  Capture is *already recorded today*: a mid-run **steer** is a correction
  with full context in the session JSONL, an approval **denial** is a
  recorded rejected intent, and a corrective chat turn after an action is the
  third form. Metric: steers, denials and corrections per session should fall
  over time — observable from the transcripts alone.

Both run the same three-stage lifecycle, and the stages are distinct on
purpose:

1. **Reflection** — at the moment of correction, generate one short
   contextual note: what mecha was doing, what the user changed or stopped,
   what they evidently wanted. Raw, per-incident, cheap. Stored with a
   context snapshot in a **mecha-local store beside the sessions, not pkg**
   (high volume, its own lifecycle, procedural rather than queryable — the
   one deliberate amendment to the one-graph rule).
2. **Abstraction** — every ~N reflections per domain, an LLM pass extracts
   durable candidate rules (flowmail's schema ports: `source='learned'`,
   confidence, `based_on_count`). Reflections are archived, not deleted —
   they are the evidence trail and the held-out set.
3. **Consolidation** — the stage that keeps it honest about context: the
   rule set has a **fixed token budget per domain**. When abstractions
   accumulate, a consolidation pass merges overlapping rules, drops
   superseded ones, and compresses — so learning never grows the system
   prompt without bound. Rules change only at consolidation time, which also
   keeps the prompt-cache prefix stable between passes.

Prompt assembly order: the user's own rules → consolidated rules → a small
sliding window of recent raw reflections. Everything inspectable, editable,
disableable; the loop is self-sealing (a bad rule → a bad action → a
reflection that fixes the rule). **Measure, or it isn't learning**: hold out
a slice of reflections at each abstraction pass and check the rules move the
metric (edit distance for writing, intervention rate for behavior) on data
they did not train on. A pass that does not beat its control is prompt
clutter, and this rig knows what to do with that. Bridge to pkg: rules that
are really facts about the user ("signs off 'Cheers' with lab members")
graduate to staged pkg facts.

Build order: hooks (capture) → reflection store + reflection generation →
abstraction pass → consolidation with the token budget → held-out
measurement. The outbox slots in as the email capture point when it lands;
the behavior system needs nothing the harness does not already record.

**`pkg` as memory.** Wired, and the design is now settled — see item 3, which
supersedes the turn-start retrieval idea this paragraph used to propose
(turn-start retrieval arms the trifecta at turn zero and was rejected for it).
The staging queue remains the guardrail: a self-learning agent that cannot
silently poison its own memory. **Do not build a second memory store beside
it** — provenance scoping (`source: agent:mecha`) inside the one graph is the
separation that matters.

**Triggers.** Cron, file watchers, inbound webhooks. Sandboxing exists now, so
this is unblocked.

**TUI polish.** The `todo` list is not a live pane, and nested subagent calls
render flat rather than as a tool-call tree.

The TUI now has slash commands (`/help /tools /model /provider /mode /mcp
/usage /clear /session /exit`) and can switch model, provider, permission mode
and MCP servers mid-session. Three rules that switching had to respect, each of
which is a bug if forgotten:

- **Switches compose.** They rebuild from the options currently in force, held
  on `Live`, not from the flags the process launched with. Rebuilding from the
  launch flags means `/mcp off` followed by `/model x` quietly turns MCP back
  on.
- **Every switch appends a `Record::Config`**, with the *live* permission mode
  rather than the file's — a replay reading the file would reproduce
  permissions the session never ran under.
- **A switch is idle-only, and taint does not un-happen.** Dropping the servers
  that fetched something hostile does not unread it; only `/clear` resets
  taint, because only `/clear` drops the context too.

Still to build: `ask_user` (a tool that blocks on a human, reusing the approval
modal's plumbing) and phase-gated planning. Together those are what turns
`/mode plan` from a permission label into a real planning phase — and `ask_user`
would let the `ambiguity` cases be graded on the trace instead of by a judge,
which is the rig's weakest and noisiest tag.

### 6. Open security gaps

- **`mecha run` used to record no taint at all**, which meant `--resume` on a
  one-shot laundered it. Fixed, but the shape of that bug is worth remembering:
  the guarantee was implemented in two of three front-ends and nobody checked
  the third. When something is enforced per-interface, enumerate the interfaces.
- **Evals inherited whatever MCP servers were configured locally** until
  `--mcp` made them opt-in. A scorecard that depends on today's machine is not
  comparable to yesterday's. Adding pkg to config silently changed the tool
  surface mid-experiment and invalidated an A/B in flight.

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
`"max_turns": N` gives a case its own turn budget; `"compact_at_tokens": N`
forces compaction for one case alone; `expect.verify` runs a command in the
case's workspace afterwards and grades the exit code; `expect.judge` grades a
rubric with a second model (`--judge-provider`, `--judge-model`).

`"prompt"` takes a string *or* a list of strings. A list runs on one
`Conversation`, which is what makes anything cross-turn expressible at all —
taint accumulating, a transcript growing past the compaction threshold. It is
untagged in serde, so no existing case changed. `RunContext` gained
`compact_at_tokens` for the same reason it already carried the budget and the
jail: one agent serves many runs, and a case that means to exercise compaction
must not force every other case to compact too.

`expect.stop_cause` / `taint` / `blocked_sends` / `min_compactions` grade the
*harness* rather than the model. None of it is visible in the answer text.
`min_compactions` exists specifically so a compaction case fails loudly when
the transcript never crossed the threshold, rather than passing and reporting
fidelity it never tested.

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
- **What the dropped future takes with it.** Not just the text — the usage
  frame too, which is why an interrupted run used to report **zero** tokens
  after spending them. Providers now emit `StreamEvent::Usage` as counts
  arrive, and the loop keeps them in the same place it keeps the partial text:
  outside the future. Input is known from the very first frame, which is the
  expensive half when a cached prefix is in play. The cut turn's *output* is
  still unknown, so `RunOutcome::usage_complete` is false and the CLI prints
  "at least" — a floor that admits to being one, rather than a guess dressed as
  a measurement in the same field a cost budget reads.
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
- **Fidelity is not legality, and only one of them is unit-testable.** The cut
  points are pure functions with tests. Whether a summary carried the task
  forward can only be answered by a model that had to use it.
- **Measured, and it is worse than this file used to claim.** Two cases, same
  model, same threshold, on 2026-08-03:

  | Case | Result |
  |---|---|
  | `compaction-carries-the-task` — recall a token stated in turn 1 after 8 filler turns | **3/3** |
  | `chain-total-compacted` — the 16-link traversal, `compact_at_tokens: 1200` | **1/5** |
  | `chain-total` — the identical task, uncompacted | **5/5** |

  5/5 against 1/5 on the same task with one variable changed (Fisher's exact
  p≈0.05). The earlier claim in this file — "it compacted four times and still
  answered 16 entries / 847" — was one sample and does not hold up.

  The failure mode names the cause. The two logged walks lost their *place*,
  not their facts: one invented `next: END` five links early, the other read 14
  links correctly, re-read an entry it had already seen, and restarted from
  `START.md`. Meanwhile a stated fact survives compaction 3/3.

  So the summariser preserves **what is true** and drops **how far you got**.
  Read `SUMMARY_INSTRUCTION` with that in mind: it asks for established facts
  with their values, for what failed so it is not repeated, and for what
  remained — but never for position in a sequence, and "which entries I have
  already visited" is neither a fact about the world nor a failed attempt.

  Two things were tried. Measured on qwen3.6-35b-a3b at `compact_at_tokens:
  1200`:

  | arm | `chain-total-compacted` | `carries-the-task` |
  |---|---|---|
  | original summariser | 1/3 | 3/3 |
  | + a clause asking for traversal position | 2/5 | 5/5 |
  | + tiered thinning | **4/5** | 5/5 |
  | + todo instruction, prompt only (2026-08-04) | 4/5 | 5/5 |
  | + todo instruction, prompt + tool description (2026-08-04) | 4/5 | 5/5 |
  | uncompacted control | 5/5 | — |

  The two todo arms are not really separate treatments: the model never called
  `todo` inside the eval in either one, so both are further samples of the
  thinning arm — which pools to **12/15**, and every failure in the pool is a
  wrong *total* over a correctly-completed walk. See item 1 of "What to do
  next" for the full finding.

  **The prompt clause did nothing** (1/3 → 2/5 is noise). **Thinning appears to
  close most of the gap**, but be careful with that number: 4/5 against the
  pooled 3/8 of both earlier arms is p≈0.27, which is not significance at n=5.
  What makes it more believable than the clause is not the p-value but the
  mechanism — the claim is "the sequence of tool calls survives", and that is a
  unit test rather than a hope about what a summariser noticed. Run n≈15 per arm
  if the number needs to be citable.

  The design is in `thin_old_results`: a call and its result differ enormously
  in size *and* value, so shorten the results and keep the calls. Position stops
  being something a summary has to preserve and becomes something the transcript
  structurally still contains.

  The todo-list idea was tried and measured on 2026-08-04 — the model ignores
  the instruction, and the position-loss mode it targeted is already fixed by
  thinning. The finding, with mechanism, is item 1 of "What to do next".

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
- **A test can pass for a reason you did not write.** The first version of the
  broken-sandbox test named `image:` before `..cfg` in a helper, so Rust's
  struct-update ordering silently overrode the caller's deliberately broken
  image and the test ran against a working one. It failed only because it
  *passed* when it should not have. Put the forced fields where they cannot
  shadow the caller's, and comment why.
- **A negative assertion needs a machine where the positive holds.** The
  confinement tests assert no network and no `~/.ssh`; both would pass
  vacuously on a host that had neither. Check the host has them before
  believing the sandbox took them away.
- **Read the transcripts before believing the score.** The clean A/B on
  `ask_user` said it made no difference: 6/9 either way. The transcripts said
  otherwise, and they were right — without the tool the model burned **30 tool
  calls** and died on the turn ceiling *with a correct answer*; with it, it
  asked in **3** and failed a rubric demanding it ask for two missing things at
  once. Identical scores, opposite reasons, and a large real improvement
  invisible to the grader. Rewriting the cases to assert on the trace
  (`tools: ["ask_user"]`, and `forbid_tools: ["ask_user"]` where asking is the
  *wrong* move) took the tag from 6/9 to 8/9.
- **A tool's failure text is part of its behaviour.** `ask_user`'s decline
  originally said "proceed with your best interpretation", and the model duly
  invented a contractor name and rate — precisely the failure the case exists to
  catch. A decline must not read as permission to guess.
- **Changing the tool surface invalidates the comparison, including
  accidentally.** Adding pkg to `~/.mecha/config.toml` gave every eval case five
  extra tools in the middle of an A/B. Same trap as the fixture change, entered
  from a different door.
- **An override that widens should not narrow something unrelated.** Forcing
  `untrusted_input` on an MCP server also revoked `read_only`, on the reasoning
  that a distrusted tool should not skip the approval gate. But distrusting what
  a tool *returns* says nothing about whether it *writes* — and the result was
  every memory retrieval demanding approval. Only a forced `destructive`
  contradicts read-only.
- **Grade the control before believing the treatment.** `chain-total-compacted`
  failed on its first run, which looked like a compaction finding — until the
  uncompacted control failed too, which made it look like nothing. Both readings
  were n=1. Five runs of each turned it back into a finding, and a real one.
  A case that isolates one variable proves nothing while its control is
  unmeasured, in *either* direction.
- **Do not couple a diagnostic to your hardest case.** The first compaction
  case was the 16-link traversal, so a compaction failure and a long-horizon
  failure were indistinguishable in the result. The replacement states a token
  in turn one and asks for it after eight filler turns: the underlying task is
  trivial, so the case can only fail for the reason it is named after.
- **Assert on the trace, not only on the answer.** `chain-total` was graded on
  its total, so a model that stopped early and summed its own truncated list
  *correctly* failed with "847 not in the answer" — true, useless. Asserting it
  read `entry-d084.md`, the one real terminator, names the failure instead. It
  also closed a hole in `chain-largest`, whose answer sits at link 9 of 16 and
  could be reached without ever finishing the walk.

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
