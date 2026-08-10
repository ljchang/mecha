# mecha — handoff

Where the project stands and what is actually left to do. Written to be picked
up cold.

Two companion documents, so this one can hold open work and nothing else:

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
The three crates are on **crates.io at 0.1.0** as of 2026-08-08 (`mecha-core`,
`mecha-mail`, `mecha-cli` — the bare name `mecha` was taken, so the CLI crate
installs the `mecha` binary), published through the tag-driven `release`
workflow with Trusted Publishing, so no registry token exists anywhere. The
next repo release is **v0.1.1**: the v0.1.0 GitHub release predates the
crates, and tags converge with crate versions from 0.1.1 on. How work lands —
branch per arc, one worktree per session, PR-gated, release by tag — is
`CONTRIBUTING.md`'s to state.
CI runs build, test, clippy and rustfmt on every push and pull request; the
documentation site builds from `website/` and deploys to
<https://docs.mecha-factory.ai/> — GitHub Pages still hosts it; the custom
domain is asserted by `website/static/CNAME`, which ships inside the deployed
artifact because `actions/deploy-pages` writes no such file itself.

The site has a **Factory** section as of 2026-08-08, opening with a live
component gallery at `/docs/factory/gallery`. Its frames are the real pages
generated in mecha-factory (`cargo run --example gallery`), committed there at
`gallery/` and copied in at build time by `website/scripts/sync-gallery.mjs` —
sibling checkout first, public tarball otherwise, warning rather than failing
so the prose still builds offline. The copy is gitignored here; that repo's CI
owns the drift check, because that is where the renderer lives. Nine more
pages are planned in [`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md).

Every commit was verified to build and pass tests **in isolation**, so the
history bisects rather than merely ending in a good state.

First thing to run in a fresh context:

```bash
cargo test --workspace && cargo clippy --all-targets --all-features
```

Expect **688 tests**, no failures — verified 2026-08-09, after the day's three
arcs (counts re-measured at the end of the session): inter-agent messaging (`mecha-core` grew with the mailbox store,
taint-forwarding, and the review's fix tests), the benchmark-diagnosis fixes
(overflow-recovery, empty-turn, and session-rewrite regression tests, including
the review-caught rewrite-drops-stale-taint-positions one), and the Slack
transport with its binding store and thread state machine. One flake was seen
once in `mecha-core` on 2026-08-08 and never reproduced across five re-runs —
unidentified, worth an eye.

| Suite | Count |
|---|---:|
| `mecha-core` unit | 379 |
| `mecha-cli` unit | 141 |
| `mecha-mail` unit | 86 |
| `mecha-slack` unit | 68 |
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
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep ; `slack` on a branch (PR #25) |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning. Review lives here too: `/outbox` and `/frontdoor` modals drive the CLI like `/triggers` does, the status line badges pending drafts, and `/review now\|later\|auto` decides what happens when a run stages some — scoped to that run's items by an id-diff, tainted drafts never auto-released, the mode set only by command (never parsed from the prompt). Detached releases/extractions/triages are watched and their results reported without a reopen |
| Slack | `mecha slack` — a remote control: Socket Mode from home, an owner allowlist bound by a locally printed nonce, a thread as a `Conversation`, streamed answers with a task card per tool call, approval cards (incl. "allow for this run"), outbox review cards, files both ways, `notify`. **On `slack/transport`, not merged** |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit/**review**/send/reject, several ids or `--all` narrowed by `--kind`/`--via`; edits mined as writing reflections. Items carry a kind — a publish shows its rendered page, refuses `edit`, and is excluded from the miner — and the jail they were drafted under, so a release resolves paths against the agent's workspace rather than the reviewer's |
| Messaging | `[messages]` + `mecha msg send/list/show/dismiss/agents` — a file mailbox between this machine's sessions (`~/.mecha/messages/<recipient>/`, producer-name addressing, per-session liveness registry). Delivery folds in at the steering point with the sender's harness-stamped taint merged first, so a hop launders nothing; attended surfaces hold with a notice, unattended accept; global config only; full mailboxes refuse, identical pending sends dedup. `docs/MESSAGING-RESEARCH.md` is the design record; phase 2 (TUI modal/badge) is scoped there |
| Workspaces | `~/.mecha/work/<producer>/` is a run's workspace and its output directory; `mecha work list/path/clean`, retention nightly. A workspace containing the mecha home is refused |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed |
| Front door | `mecha frontdoor` list/show/extract/next/**triage**/**needs-info**/**close** over `~/.mecha/requests/` — the quarantine between a stranger's request and a run with tools, and the state machine that lets one reach an answer. The extractor is issued no tools and no history; `Record::for_privileged_run` has no argument that returns the prose; an extraction failure routes to a human. `triage` drafts into the outbox and refuses to run unrouted; `reconcile` closes the loop from released items on its own, with no verb to remember. `mecha-factory-publish drain` fills the directory |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, **the daemon is installed and running here**; a failed `notify` is recorded on the run |
| Learning | the full arc: reflect-on-close → nightly rumination → counterfactual validation (steers/denials trace-graded) → gated proposals (`mecha proposals`); git-backed store under `~/.mecha/learning`; rules carry id/sources/created_at, validate feeds a per-rule outcome ledger with regression bisection, and `mecha rules` retires through the same gate (`eval --ab-rules` for the coarse A/B) |
| Eval | 36 cases, 15 tags, scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `pkg-cases.jsonl` — 8 memory/interlock cases against fixture MCP servers (`--mcp-file`) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Re-verified
2026-08-08 night** (8080 answering with `total_slots=1`; 8082 down; the mecha
units enabled as listed below — 8081 and the binary dates carried forward
from the afternoon pass, not re-checked):

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, `total_slots=1`, `-c 32768`, **`--reasoning-budget 4096`** (new 2026-08-07 — it *reduces* this model reasoning without terminating and returning empty content, but the 08-07 benchmark run proved it does not end it: empty turns persisted with the budget active, which is why the loop's nudge-retry allowance now resets on productive turns). `~/.mecha/config.toml` and `bench/mecha_agent.py` carry `context_window` (= `-c`) and `max_tokens` (**above** the budget; 8192) — four numbers that move together. **Do not raise `-c`**: 131072 was tried and cost a 50x generation slowdown with no error anywhere. MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`). **Now a transient unit** (`systemctl --user status llama-qwen`), not a tmux pane — see below |
| 8081 | gemma-4-E4B | down; nothing currently depends on it |
| 8082 | gemma-4-26B-A4B | **down — restart it before any judged run.** The eval judge and nightly validate's judge both point here, so `mecha eval` with a `judge` rubric and the nightly validate will fail without it. `scripts/start-gemma26.sh` |
| 8888 | SearXNG | up (docker, JSON format enabled) |

**Start model servers as transient units, not from a tmux pane.** Both
llama-servers were killed on 2026-08-07 as collateral: a runaway test OOMed,
and systemd then tore down the whole `tmux-spawn-*.scope` they happened to
share (`OOMPolicy=stop`). 8080 was brought back with
`systemd-run --user --unit=llama-qwen scripts/start-moe-mtp.sh`, which puts it
outside any pane's cgroup. 8082 has not been restarted.

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
- **Booking slots refresh every two minutes, and the drain is a live loop.**
  `mecha-slots.timer` (retimed from fifteen minutes, 2026-08-08 evening —
  the calendar→box window was the one real double-booking risk) → freebusy
  from the named account → `factory-publish slots push`, with the
  booking-drain sweep riding the same tick as backstop. The fast path is
  **`mecha-drain.service`**, an always-on loop long-polling the box's queue
  (`drain --wait 25`) so a confirmed booking becomes a calendar event and
  its invite seconds after the click; live end to end since the box's
  v0.1.0 deploy (2026-08-08, late evening — an empty `?wait=4` poll against
  the gate held the full four seconds). The sweep takes a flock (both
  runners share it) and re-verifies each booking against live freebusy
  before creating the event; a conflict parks loudly in the ledger. The
  account is named in the units because a timer cannot ask; the loop script
  runs from `~/.cargo/bin/mecha-drain-follow`, installed like the binaries
  it drives.
- **The front door runs hourly.** `mecha-frontdoor.timer` →
  `scripts/frontdoor.sh`: drain (zero tokens, runs even with the model down) →
  extract → triage, logging to `~/.mecha/requests/logs/`. Enabled 2026-08-07
  and verified with a live tick the same hour — drain acknowledged a held
  record at the gate, and the one flagged request stayed parked for a human,
  which is the quarantine working. Triage refuses to run unrouted, so the
  `[outbox] tools` list is load-bearing for it.
- **The automation consumes `~/.cargo/bin/`, not the repo build.** Reinstall
  (`cp target/release/mecha ~/.cargo/bin/`) after changing anything in the
  learning path, and `factory-publish` too, since `morning.toml`'s `notify`
  calls it. This bit twice on 2026-08-06: once when `ruminate.sh` gained a
  `work clean` step the installed binary did not have, 41 minutes before the
  nightly, and once when a `factory-publish` fix sat unbuilt in the repo for
  nine hours, and a third time the same evening when the daemon's own `notify`
  could not see `~/.cargo/bin` at all. A fourth near-miss on 2026-08-07: the
  installed `mecha` predated the frontdoor triage verbs, caught only because
  the timer's script was checked against it before enabling. **`mecha`
  reinstalled 2026-08-07 evening** (frontdoor verbs, `StopCause::NoOutput`);
  `factory-publish` unchanged since 2026-08-06.
- The learning store (`~/.mecha/learning`) holds **zero live rules** — the one
  early rule was reverted with its poisoned reflection — so everything from here
  accumulates from real usage through the gate.

### The public box

A single small VPS runs `mecha-factory`. Its deployment posture — host, sizing,
the dedicated deploy key, the firewall policy — is inventory rather than
engineering, so it lives in `docs/OPERATIONS.md`, which is gitignored. The
deploy procedure and its traps are in that repository's `docs/DEPLOY.md` —
and as of 2026-08-08 a deploy is one command: `factory-deploy <tag>` on the
box installs a tagged, CI-built, checksum-verified static binary, proves the
binary and the config while the site is still up, and rolls itself back on a
failed health check. Hand-copied binaries are over; the outage that ended
them is in [`HISTORY.md`](HISTORY.md).

What is worth stating publicly is the invariant the posture exists to protect:
**the box holds no credential that reaches home.** Two Argon2id key hashes, the
published bytes, and a certificate. Packets go one way — mecha publishes and
drains, and the origin never dials home. That is a property you can verify by
inspection, which is why it is the one written down here.

### Provider credentials

Where the keys live is in `docs/OPERATIONS.md`. Two gotchas from wiring a key
through a shell profile, both of which cost time and neither of which is
specific to this machine:

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard near the top). An export below that line is invisible to a
  non-interactive shell, which is what tooling runs — so a key that works when
  you type a command by hand vanishes under systemd or a hook.
- **Take the *last* match, not the first**, when grepping a profile for an
  export. A placeholder above the real key meant `grep -m1` silently found the
  placeholder and produced a `401 invalid x-api-key` that looked like a bad key.

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

**As of the evening of 2026-08-07, self-serve is done.** All six steps of the
factory's `SELF-SERVE.md` are built, deployed, and verified live — a stranger
with an invite claims a handle, gets a certificate in seconds, pairs their
machine, publishes, signs in, and releases from a browser; the operator runs
their day (`factory-publish operator …`) from home. Routine SSH to the box is
over; what remains on it is deploys and disaster recovery, deliberately. The
whole arc is in [`HISTORY.md`](HISTORY.md) under 2026-08-07.

Three live checks are owed — each closes an arc that is built, deployed, and
tested, but has never been exercised against the real box by a person:

1. **The operator panel's first sign-in**: `factory-publish operator signin`,
   open the link, and `/admin` should show accounts, invites, keys and
   withholds. Two minutes, and it closes the panel arc.
2. **A real share**: grant an address you control on a private bundle from
   the viewer's Manage menu, prove the inbox from another browser, watch the
   bytes appear — then revoke and watch them stop. Closes the sharing arc.
3. **A tenant sign-in link**, whenever curiosity strikes: the gate's
   `/account` page is the operator's own tenant page. (The form-verification
   leg, once waiting here too, was run live 2026-08-08 by the booking
   self-test — `submitted → verified → queued`, drained home, acked.)

Everything else below is independent of that.

Every item below was verified against source on 2026-08-08 to still be unbuilt.
Ordered by value per unit of effort, not by size.

### The scheduling instrument — live since 2026-08-08

The booking page (`gate…/s/ljchang/book`) and the group poll are deployed and
verified in production: the full lifecycle ran live — book, email confirm,
Outlook event with the manage link in its description, cancel, native
withdrawal — with page, box and calendar agreeing at every step. The arc is
in [`HISTORY.md`](HISTORY.md) under 2026-08-08; the design authority is
[`SCHEDULING-DESIGN.md`](SCHEDULING-DESIGN.md). Open, in the order they bite:

The 2026-08-08 evening front-end pass over the whole instrument —
redesigned booking page, live `slots.json`, the poll paint grid — is
committed (`1d531a8` in that repo) and running on the box; the arc is in
[`HISTORY.md`](HISTORY.md).

- **The calendar→box freshness window is the one real double-booking risk.**
  Page-versus-page is already atomic — `booking_hold` is one INSERT-where-
  no-live-row-overlaps statement (factory `db.rs`), and the losing visitor
  gets the refreshed week; with the front-end pass, an *open tab* also polls
  `slots.json` and drops a slot the moment someone else holds it. What
  remains is the calendar side: an event landing *directly on the calendar*
  only removes its overlapping slots at the next push, up to 15 minutes
  later; `min_notice_hours = 24` makes a collision rare, not impossible. Two
  cheap fixes, complementary: tighten the timer (freebusy is one API call —
  every 1–2 minutes is free, with a hash-unchanged short-circuit so pushes
  stay rare), and teach `mecha-mail bookings` to re-verify each drained
  booking against *live* freebusy before creating the event, flagging a
  conflict loudly instead of proceeding — home always holds fresher truth
  than the box. The second closes the loop entirely.
- **The availability windows are placeholders** (Tue/Thu 13–17, Wed 9–12
  Eastern — invented, not chosen) in **two files that must agree**:
  `~/.mecha/instruments/book-policy.toml` and the `[availability]` section
  of `mecha-manifest/types/book.toml` (then re-`type push`). The page must
  not be handed to anyone before this edit.
- **Booking events land on `dartmouth` (Outlook), named in
  `mecha-slots.service`** because a timer cannot ask the account question.
  First live confusion already happened: the self-test event was "missing"
  from Google Calendar because it was never there. Switch the flag if the
  Google account should own bookings instead.
- **The vanity gate name is deferred by decision.** A redirect
  (`mecha-factory.org/book`-style) upgrades transparently to a gate-alias
  feature later; capability URLs stay minted on the real gate either way.
- **Poll polish, one piece left:** a deterministic auto-book sweep for the
  `clean_winner` case — today `polls status --json` hands the typed verdict
  to the agent, which books and closes. (The tap-to-cycle/drag-paint/heat
  layer shipped with the front-end pass; the axis-locked touch gesture and
  a separate Group heatmap tab from `SCHEDULING-DESIGN.md` §5.3 were left
  out on purpose — inline heat carries most of the value.)
- **Cosmetic:** `factory-publish type push` prints a `/f/<handle>/<id>` URL
  for booking manifests; a booking's page is `/s/…`.

### Cheap, and worth doing first

- **Re-baseline `ambiguity` and `long-horizon` at k=5.** No scorecard in
  `results/` records `runs: 5` outside the compaction arc, and these are the two
  tags whose single-run numbers move.
- **`decode_usage` reads only `prompt_tokens`/`completion_tokens`**
  (`mecha-core/src/provider/openai.rs`), so cached input reports as zero on
  every local run — the benchmark diagnosis had to reason around
  `cache_read: 0` in all 21 transcripts, and the TUI fuel gauge cannot show
  cache health. llama-server's OpenAI-shape usage carries
  `prompt_tokens_details.cached_tokens` in current builds; parse it when
  present, leave zero when absent. One function, one test.

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
  "what changed" payload injected into the prompt — the injection half got
  cheap on 2026-08-09: the mailbox delivery path (`mailbox.rs`) is exactly a
  labeled, taint-carrying prompt fold, so a watcher's payload can arrive as
  a message instead of needing new loop machinery.
- **Inbound webhooks.** Nothing listens, still. What *was* the interesting
  part — a payload arriving marked untrusted, the interlock applying to a
  prompt rather than a tool result — shipped with messaging on 2026-08-09:
  a webhook receiver is now just another writer into a mailbox with taint
  pre-set untrusted. What remains is only the listener itself and its
  authentication.

### TUI polish

- **Steering and queuing are the same key.** Enter starts a run when idle and
  steers one already going; there is no way to queue a follow-up instead.
- **No `/export` or copy.** `NAMES` lists fifteen commands and none of them
  get the transcript out.
- **`NO_COLOR` is honoured only by the plain CLI renderer.** The TUI hardcodes
  colours inline; there is no semantic colour table.
- **No keymap configuration.**

### Larger, and deliberately not started

- **`mecha-factory` — the public surface. It is deployed, it sends mail, and
  it is self-serve.** Its own repository, public at
  **github.com/ljchang/mecha-factory** (325 tests, 2026-08-08 evening),
  running on a small VPS:
  the API at `https://gate.mecha-factory.ai`, artifacts at
  `https://<handle>.art.mecha-factory.ai`, notebooks at `…compute…`. Verified
  live 2026-08-06 — a bundle rendered here, pushed there, served under the
  `static` policy behind a certificate the binary obtained for itself. Five
  scoped keys exist — `publish`, `release`, `drain`, `slots` and `operate`;
  which handles and key files exist locally is in `docs/OPERATIONS.md`.

  Everything through self-serve is **built, deployed, and verified live**:
  SES mail, the scope split that moved "a human releases" onto the credential
  (`Scope::Publish` vs `Scope::Release` — the load-bearing decision), and all
  six self-serve steps, ending with the operator running their day from
  `factory-publish operator …` and routine SSH over. The arcs, the
  verification, and the traps are in [`HISTORY.md`](HISTORY.md), the design
  in [`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) §14–15, the
  deploy procedure in that repo's `docs/DEPLOY.md`. Do not re-derive any of
  it here. One local fact worth keeping in view: `sandbox = true` is
  deliberately **not** set on the factory MCP server — bwrap does not work on
  this box, docker cannot confine the notebook render subprocess, and the
  config says why at length.

  **A second client is verified and documented (2026-08-07 evening).** The MCP
  surface was driven from a Claude Code session over raw stdio — handshake,
  `tools/list` (all `bundle_*`, no drain tool), `bundle_render` under the
  `--root` jail. `docs/SECOND-CLIENT.md` in that repo is the onboarding path.
  Still unexercised from a second client: `bundle_publish` against the live
  box (blocked mid-verification by a permission gate — that repo's
  `scripts/mcp-drive.py` is the harness, kept as the second-client smoke
  test).

  **The 2026-08-07/08 night shipped the inbound-attachments arc and the web
  face — deployed and verified live**: form attachments end to end, the gate
  chrome, scanner-proof magic links, and the signed-in artifact viewer. The
  full arcs are in [`HISTORY.md`](HISTORY.md); nothing from that night
  remains open except what the bullets below name.

  **Open there, in the order they bite:**

  - **The MCP surface has drifted behind the CLI, and nothing fails when it
    does.** `factory-publish` has twenty commands; the MCP server exposes
    eight, all `bundle_*`. Notebooks, request types (the public forms),
    availability slots and **polls** are unreachable by any agent — which is
    why mecha answered "I don't have a tool that can create polls" and was
    right. The dates make it drift rather than a decision: `mcp.rs` was last
    touched 2026-08-07, `request.rs` 2026-08-08, `poll.rs` 2026-08-09, and the
    module's long doc comment reasons only about bundles. The one *documented*
    exclusion is `drain` ("a CLI and deliberately not a tool"), and `operator`
    / `connect` / `disconnect` / `serve` belong on that list for the same
    reason — they are the operator's, not the model's.

    **Why it stayed drifted, which is the part that decides the work:** the
    capabilities were written as command bodies inside `main.rs` (the bin),
    while `mcp.rs` lives in the lib. `polls_command` alone is ~470 lines at
    `mecha-factory-publish/src/main.rs:1587`. So exposing a verb is not
    wiring — it is extracting the body into the library first, and the same is
    true of `notebook` and `type`.

    **There is no safe shortcut**, and this is worth stating because it looks
    like there is one: letting the model reach `factory-publish` through
    `shell` bypasses the outbox entirely. The outbox routes by *tool name* in
    the dispatch path — that is the stated reason the factory is an MCP server
    at all — so a shell-out has no name to route, and a poll created that way
    mints a public page and participant URLs with no review card. The prompt
    already forbids the shape ("do not try to accomplish the send some other
    way").

    Order of work: extract `polls` into the lib; expose **two** poll tools,
    because `polls create` has two mutually exclusive modes and a single tool
    with a mode flag is what a model gets wrong — `poll_create` (a general
    poll from a `--spec` TOML: choice, ranking, likert, vas, text) and
    `poll_meeting_create` (policy plus the user's real busy time, seeded from
    `mecha-mail freebusy`). The meeting one needs its freebusy as a **file
    path**, since MCP cannot pipe stdin the way the CLI does. Then
    `poll_status` / `poll_close`, then `notebook` and `type`. Creation verbs
    that mint public URLs take `openWorldHint` so they route through
    `[outbox] publish_tools`, like `bundle_publish` already does.

    **The durable fix is a coverage test**: enumerate the CLI's subcommands
    and assert each is either exposed or on an explicit exclusion list with a
    reason, so adding a command fails the build until someone decides in
    writing whether an agent should reach it. mecha already warns on every
    start when `[outbox] tools` names a tool that does not exist, for exactly
    this reason; the factory has no equivalent and that is why five
    capabilities went unnoticed.

    Costs to weigh rather than discover: every exposed tool is schema tokens
    in every run — measured at ~7–8k a turn already — which argues for
    exposing them *and* narrowing per surface (`[slack] tools`,
    `[tools] enabled`), not for leaving them out.

  - **Verify the release workflow** (`release.yml`, authored 2026-08-07:
    static musl `factory` with an asserted-static gate and a checksum). Push a
    `v*` tag and watch it; `DEPLOY.md` already leads with the
    download-and-verify procedure but says to verify the first release before
    deleting the box's toolchain.
  - **The crates.io split.** Both `mecha-manifest` and `mecha-factory-publish`
    `cargo package` and verify cleanly (checked 2026-08-07, including the
    packaged-dependency resolution). What remains is `cargo publish` with the
    owner's token — claiming the names is forever, so it stays a human's
    button to press.
  - **The apex redirect is deployed but dormant, waiting on DNS.**
    `redirect_hosts` (301 to the gate, path kept, riding the base
    certificate) is in the deployed binary; the config line was deliberately
    removed because the apex still points at the registrar's parking
    servers, and an ACME challenge for a name not resolving here would jam
    renewal for the real origins. Sequence: point apex and `www` A-records
    at the box, then add
    `redirect_hosts = ["mecha-factory.ai", "www.mecha-factory.ai"]` to the
    deployed `factory.toml` and restart.
  - **The DNS is at a registrar with no API for custom records** — every
    row is typed by hand, including the five SES ones. Moving the zone to
    Cloudflare (DNS-only, never proxied) is independent of everything else —
    and it gates nothing: the wildcard `A` records already resolve any new
    handle, which is what let per-user issuance ship without touching DNS.
    It does **not** unlock wildcard certificates either: `rustls-acme` speaks
    only HTTP-01 and TLS-ALPN-01, so the library forecloses them whoever
    hosts the zone.
  - **The factory documentation is half written.** Shipped: the component
    gallery, `polls.md`, `slides.md`, and — 2026-08-10 — `onboarding.md`
    (claiming a handle, pairing, the five scoped keys, wiring the MCP surface
    into an agent), `artifacts.md` (versions, the alias, visibility, the
    share/revoke path and its oracle rules, takedown, what retention will not
    sweep) and `notebooks.md`. Also `features/slack.md` on the mecha side,
    which is setup plus what the remote control actually does.
    [`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md) lists the rest with the
    claims each has to make, sourced from the code. Still missing, in the order
    they bite: `field-kinds.md` — the four-column table (TOML · JSON Schema ·
    rendered control · what the server enforces) exists nowhere but
    `request.rs`, and `second-client.md` assumes it; `booking.md`, since the
    whole scheduling instrument is still undocumented for readers;
    `overview.md`, `request-types.md` and `theming.md`.

    Note for whoever writes the next one: the site's `sync-gallery` step
    fetches **mecha-factory@main**, so a gallery fix on a branch does not show
    up in a local docs build until it merges.
  - **The operator admin panel and private sharing are built, reviewed,
    tested, and deployed** (2026-08-07/08; the arcs are in
    [`HISTORY.md`](HISTORY.md)). What remains is the two live checks at the
    top of this section — nothing else is open on either. The one fact
    worth keeping in view here: the box now runs three session surfaces
    (tenant / operator / reader), deliberately parallel and never unified —
    `Db::signin`'s doc comment records why, so do not "deduplicate" them.

  The mecha-side half of step 7 is **built**: `mecha-factory-publish drain`
  fetches the queue, and `mecha frontdoor` is the quarantine between a drained
  record and a triage run. See `CLAUDE.md`.

- **Slack as a remote control — built and verified live; two things left.**
  The arc is on branch `slack/transport` (**PR #25**) and is described in
  [`HISTORY.md`](HISTORY.md) under 2026-08-09; the design authority is
  [`SLACK-DESIGN.md`](SLACK-DESIGN.md) and the evidence
  [`SLACK-RESEARCH.md`](SLACK-RESEARCH.md). What is genuinely unbuilt:

  - **`ask_user` is absent, and the reason is structural.** The approver rides
    on `RunContext`, so it is per-thread for free; `ask_user` is a *tool* and
    the registry belongs to the `Agent`, one of which serves every thread, so
    a shared `AskUserTool` cannot know which thread asked. Routing it needs an
    agent per thread (an MCP startup each) or a registry per run. The second
    is the smaller change and would also close the item below.
  - **MCP tools do not honour the per-thread jail** — only the built-in tools
    do, because servers are spawned once with the agent. They are rooted at
    the `slack` producer directory so paths at least agree; isolation between
    threads is not there, and closing it is the same fix as above.
  - **The outbox review cards have not been exercised live.** Built and unit
    tested; no run has yet staged a draft while the connector was watching.
  - **It is installed and running** (2026-08-09). `mecha-slack.service` is
    enabled with linger, `~/.cargo/bin/mecha` carries the merged binary, and
    `[slack] tools` is set — workspace tools, `web_search` + `http_fetch`
    (which the `research` subagent needs, and whose absence silently
    unregisters it), and the factory bundle tools. `mail__*` and `pkg` are
    deliberately out: largest schemas, most private surface, one line to add
    when inbox work from a phone is actually wanted. The connector answers a
    **shared lab workspace** (`cosanlab`) rather than the personal one §11.1
    of the design chose — safe, since non-owners are ignored by construction,
    but the app is visible to its members.

- **The factory must never become an owner channel.** Recorded here because the
  reuse is tempting and wrong: `GET /v1/queue?wait=` plus `mecha-drain.service`
  is exactly the right-shaped channel, but hosting the Slack app on the box puts
  a workspace credential on a machine the design assumes is lost, and a command
  queue inverts the direction of *authority* even while preserving the direction
  of packets. Anything arriving from the box stays a request, not a command.
  What the factory should carry is artifacts too large for Slack — the split is
  "Slack carries control, the factory carries bytes."
- **Public benchmarks.** The Terminal-Bench adapter (`bench/`) is written, and
  the **oracle arm64 sweep is complete** (2026-08-05, 14.4h): 75 of 89 tasks
  have a reference solution that passes on aarch64, and those 75 are the only
  comparable set — `bench/oracle-arm64-excluded.txt` holds the other 14.
  `bench/run-subset.sh` runs the calibrated subset. **No complete scorecard
  exists yet**: the 2026-08-07 05:22 launch was voided by the glibc trap, and
  the 11:18 relaunch (portable binary, verified to be exactly the 75) was
  stopped by hand ~4h in to free the box — the salvaged fragment is 21 trials,
  8 solved, in `jobs/mecha-arm64-subset/`. That fragment has now been
  **diagnosed trial by trial** (2026-08-09 — the write-up is in
  `docs/BENCHMARK-RESEARCH.md`, "The 2026-08-07 subset run, diagnosed"): of
  13 failures, 5 were the model and 8 involved harness defects, all five of
  which are fixed with regression tests (PR #21 — overflow-recovery
  poisoning, cumulative empty-turn allowance, exit-3-on-exhausted read as an
  agent crash, transcripts lost on crash or corrupted by compaction, and a
  flat output budget bigger than the threshold-to-window gap). Relaunching
  the full 75 at k=1 (~15h) is still the open decision, and it is now worth
  doing: the fixed binary should recover most of the 8. Rebuild the portable
  binary (`bench/build-portable.sh`) from the merged branch first — the
  installed one predates every fix, and `bench/run.sh` resolves
  `$(pwd)/target-musl/release/mecha`, so the binary scored is whichever
  checkout you launch from. Consider `--agent-timeout-multiplier` too: the
  two timeout deaths were 12.5- and 15-minute per-task caps against local
  inference, tight even with the empty-turn waste fixed. Read any job with
  `bench/check-subset.py <job>` first — a harbor `-x` that matches nothing is
  silent, and two earlier runs scored all 89 while claiming to be a subset.
  k=5 for a leaderboard-comparable number is the follow-up, ~74h.
  AgentDojo (for the interlock) and a SWE-bench Bash Only control are named in
  the research and unstarted.
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
