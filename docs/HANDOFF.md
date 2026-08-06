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

Expect **491 tests**, no warnings — verified 2026-08-06:

| Suite | Count |
|---|---:|
| `mecha-core` unit | 331 |
| `mecha-cli` unit | 80 |
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
| Context | Two-pass compaction: thin tool results, then summarise. Taint preserved, and a tool's own state (the todo list) crosses verbatim |
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit/**review**/send/reject, several ids or `--all` narrowed by `--kind`/`--via`; edits mined as writing reflections. Items carry a kind — a publish shows its rendered page, refuses `edit`, and is excluded from the miner — and the jail they were drafted under, so a release resolves paths against the agent's workspace rather than the reviewer's |
| Workspaces | `~/.mecha/work/<producer>/` is a run's workspace and its output directory; `mecha work list/path/clean`, retention nightly. A workspace containing the mecha home is refused |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, **the daemon is installed and running here**; a failed `notify` is recorded on the run |
| Learning | the full arc: reflect-on-close → nightly rumination → counterfactual validation (steers/denials trace-graded) → gated proposals (`mecha proposals`); git-backed store under `~/.mecha/learning`; rules carry id/sources/created_at, validate feeds a per-rule outcome ledger with regression bisection, and `mecha rules` retires through the same gate (`eval --ab-rules` for the coarse A/B) |
| Eval | 36 cases, 15 tags, scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `pkg-cases.jsonl` — 8 memory/interlock cases against fixture MCP servers (`--mcp-file`) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Re-verified
2026-08-06, end of session:**

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, `total_slots=1` — MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`) |
| 8081 | gemma-4-E4B | down; nothing currently depends on it |
| 8082 | gemma-4-26B-A4B | up, `total_slots=1` — the eval judge and nightly validate's judge |
| 8888 | SearXNG | up (docker, JSON format enabled) |

**`-np 1` is load-bearing**, and the check before believing any measurement is
`curl :8080/props | jq .total_slots` — it must be 1. The build in use defaults
to 4 parallel slots and silently splits `-c` across them; the story of what
that cost is in [`HISTORY.md`](HISTORY.md) under Traps → Environment.

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
  `--holdout 0.25 --propose` → `rules propose-retirements` → `work clean`.
  Logs land in
  `~/.mecha/learning/logs/<date>.log`; pending proposals wait in
  `mecha proposals`. **Confirmed enabled 2026-08-05.**
- **The trigger daemon is installed and running.** `mecha-triggers.service`
  (systemd user, linger on), enabled 2026-08-06 and confirmed firing on its own.
  `morning.toml` is jailed to `~/.mecha/work/morning`, and its `notify` writes
  the briefing then renders it. A failed `notify` is recorded on the run, so a
  briefing that has quietly stopped rendering does not read as a healthy one.
- **The automation consumes `~/.cargo/bin/`, not the repo build.** Reinstall
  (`cp target/release/mecha ~/.cargo/bin/`) after changing anything in the
  learning path, and `factory-publish` too, since `morning.toml`'s `notify`
  calls it. This bit twice on 2026-08-06: once when `ruminate.sh` gained a
  `work clean` step the installed binary did not have, 41 minutes before the
  nightly, and once when a `factory-publish` fix sat unbuilt in the repo for
  nine hours, and a third time the same evening when the daemon's own `notify`
  could not see `~/.cargo/bin` at all. **Both verified installed 2026-08-06
  13:35**, after the trigger and compaction work.
- The learning store (`~/.mecha/learning`) holds **zero live rules** — the one
  early rule was reverted with its poisoned reflection — so everything from here
  accumulates from real usage through the gate.

### The public box

[redacted: operational detail — see docs/OPERATIONS.md]
`mecha-factory`. Reached over SSH with a **dedicated** key at
[redacted: operational detail — see docs/OPERATIONS.md]
[redacted: operational detail — see docs/OPERATIONS.md]
[redacted: operational detail — see docs/OPERATIONS.md]
[redacted: operational detail — see docs/OPERATIONS.md]
would be.

The box holds no credential that reaches home: two Argon2id key hashes, the
published bytes, and a certificate. Packets go one way — mecha publishes and
drains, and the origin never dials home.

### The Anthropic key

[redacted: operational detail — see docs/OPERATIONS.md]

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard near the top). The export sits well below it, so a non-interactive shell
  — which is what tooling runs — never reaches it. Load it explicitly:
[redacted: operational detail — see docs/OPERATIONS.md]
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

Two things a reader needs before trusting any number here, both with the detail
in [`HISTORY.md`](HISTORY.md) under "The measurement record":

- **The original 25-case set is saturated** — all four local models score 23–24
  of 25. It is a floor test, not a ranking test, and it stays in the file as
  exactly that. The tags added since (`long-horizon`, `codegen`, `synthesis`,
  `ambiguity`) do discriminate; qwen3.6-35b-a3b scored 32/34 on the set as it
  stood (`results/qwen-hard-v2.json`).
- **Scorecards taken before the fixture expansion are not comparable to ones
  after it.** The shared workspace went from 11 files to 44, so every case that
  searches it got harder. If you add fixtures, re-baseline every model rather
  than comparing across the boundary.

Everything a judge touched — `ambiguity` and `synthesis` — is worth reading the
answer for rather than trusting the verdict, because judges disagree with
themselves across runs.

---

## What to do next

**Sequenced**, and the ordering changed on 2026-08-06 because the factory
stopped being hypothetical. It is deployed, reachable, and — since the wiring
landed the same day — reachable *by an agent*. The outbound half is done; the
inbound half is what is left.

So the short path, in order:

1. **The drain client at home.** The server half exists (`GET /v1/queue` with a
   scoped key, `~/.mecha/factory/drain.key` already minted), and nothing on
   this side speaks it: `mecha-factory-publish`'s `remote.rs` knows only
   bundles and health, and there is no `drain` subcommand. It is a CLI and not
   a tool on purpose — the common case is "nothing new", and it must cost zero
   tokens.
2. **The quarantine layers (§8)**, which are what stands between a drained
   stranger's record and a triage run. They are the mecha-side half of factory
   step 7, and nothing about the inbound path is safe to switch on without
   them.
3. **A mailer**, after which a stranger can actually use a form.

Everything else below is independent of that path.

Every item below was verified against source on 2026-08-06 to still be unbuilt.
Ordered by value per unit of effort, not by size.

### Cheap, and worth doing first

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

- **A durable task and deadline store, and a `/tasks` TUI modal.** Nothing
  tracks these: `~/.mecha/` has no task store and the `todo` tool is an in-run
  scratchpad that dies with the run. This is what turns silence from an absence
  into a state that can be surfaced and escalated — an unanswered message is
  currently invisible because there is no object to hang a state on. Three
  sources: an inbound request's SLA, a **commitment the user made** (extractable
  from released outbox items, where mecha already knows what went out), and
  direct capture. Stored in pkg with an `Origin` per task so only user-origin
  tasks escalate unattended; recurrence reuses `cron.rs`. The modal drives the
  CLI like `/triggers` does. Design in `PUBLIC-SURFACE-DESIGN.md` §3.2–3.3.
- **Policy questions as a new `proposals` kind — not a third queue.**
  `ask_user` is absent from unattended runs by construction, so a trigger can
  stage but cannot ask. The elicitation that grows autonomy is *policy*
  questions that unblock a class, not per-item approvals. These belong in
  `mecha proposals` beside `rule` and `retirement`, because it is the same
  shape — mecha asks, the user decides, future behaviour changes — and because
  two review surfaces (`outbox` for what leaves, `proposals` for what changes
  behaviour) is a learnable morning routine where three is not. Deliberately
  **not** called an inbox: ambiguous with the user's real one, and this queue is
  capped by design where an inbox is unbounded by definition. §3.1.
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

- **`mecha-factory` — the public surface. It is deployed.**
  Its own repository, public at **github.com/ljchang/mecha-factory** (187
[redacted: operational detail — see docs/OPERATIONS.md]
  `https://gate.mecha-factory.ai`, artifacts at
  `https://<handle>.art.mecha-factory.ai`, notebooks at `…compute…`. Verified
  live 2026-08-06 — a bundle rendered here, pushed there, served under the
  `static` policy behind a certificate the binary obtained for itself. The
[redacted: operational detail — see docs/OPERATIONS.md]

  Steps 1–7's server half are built: publishing, multi-tenancy (a user owns
  every row, artifacts are served from their own hostname, handles are never
  reissued, withhold/suspend/quota), and the intake path (a form from the
  manifest, a single-use verification link, nothing queued until somebody
  clicks). The design is [`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md)
  §14–15; the deploy procedure and its traps are that repository's
  `docs/DEPLOY.md`. Do not re-derive either here.

  **Wired into this machine 2026-08-06**, and verified by publishing through
  it: an agent rendered a page, `bundle_publish` staged instead of executing,
  `outbox show` led with the rendered file, `edit` was refused, and `send`
[redacted: operational detail — see docs/OPERATIONS.md]
[redacted: operational detail — see docs/OPERATIONS.md]
  change for that to work, both in the "Traps" record: an unconfined MCP server
  now starts in the run's workspace, and a staged item records the jail it was
  drafted under. `sandbox = true` is deliberately **not** set here and the
  config says why at length — bwrap does not work on this box, docker would
  need a custom image plus global writable mounts, and neither confines the
  notebook render subprocess, which is the one path that executes code we did
  not write.

  **Three things open, in the order they bite:**

  - **No mailer.** Verification links go to the journal rather than to anyone.
    It is the gap before a stranger can use a form at all, and §15.5 is the
    shape it has to take.
  - **A release binary from CI**, so the box needs no Rust toolchain to patch.
  - **A wildcard certificate** (needs DNS-01, and a zone token on the box).
    Until then a new user needs a restart to get one — and note what that buys
    today: an unowned handle fails at the TLS handshake, so the 404 is only the
    second line of defence and a wildcard would remove the first.

  The mecha-side half of step 7 — §8's quarantine layers — is unbuilt, and is
  what stands between a drained record and a triage run.

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
