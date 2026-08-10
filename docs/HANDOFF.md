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

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.2**
(2026-08-10 evening — the reasoning round trip; 0.1.1 was earlier the same
day). **Four** crates are on crates.io at 0.1.2 — `mecha-core`,
`mecha-mail`, `mecha-slack`, `mecha-cli` (the bare name `mecha` was taken, so
the CLI crate installs the `mecha` binary) — published through the tag-driven
`release` workflow with Trusted Publishing, so no registry token exists
anywhere. From v0.1.1 on, a tag and a crate version are the same number.

`mecha-slack` joined at this release, and the reason is worth keeping because
it generalises: **cargo refuses to publish a crate whose non-dev dependencies
are not on the registry**, and `mecha-cli` depends on it. Tagging without
adding it to the workflow's list would have published `mecha-core` and
`mecha-mail`, then failed on `mecha-cli` — a half-published version that can
be yanked but never unpublished. A new workspace member that anything
published depends on belongs in that list in the same change that introduces
it. Its first publish was by hand, because Trusted Publishing can only be
configured on a crate that already exists; TP was added for it immediately
after.

How work lands — branch per arc, one worktree per session, PR-gated, release
by tag — is `CONTRIBUTING.md`'s to state.
CI runs build, test, clippy and rustfmt on every push and pull request; the
Claude review bot runs beside them, and the `@claude` mention bot answers on
issues and pull requests. **Both were repaired on 2026-08-10 night and had
never worked before it** — the review job had failed on every run it ever
made (a credential, on both sides of the OAuth migration that was itself an
attempt to fix it) and the mention job's condition could not be true. Both now
pin `--model claude-opus-5`; without a pin the action runs `claude-sonnet-5`.
Two things about them that will otherwise cost somebody an afternoon:
`claude-code-action` **skips and exits 0** when the workflow file differs from
the copy on `main`, so a workflow change can never be tested by its own pull
request and a green check there means nothing; and the same workflows are now
in mecha-factory but **inert until the Claude GitHub App is installed on that
repository** — the token is not enough, and the job fails with
`Claude Code is not installed on this repository`.
The documentation site builds from `website/` and deploys to
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

Expect **707 tests**, no failures — re-measured 2026-08-10 evening, after the
reasoning round-trip arc added seventeen (`reasoning_content` decode and
re-encode, `produced_output` versus block count, the cached-token split and its
underflow guard, the salvage of a reasoning-only run, and the lost-call marker
across four model families). Before that it was 690, from 2026-08-09 night and
that day's
four arcs (counts re-measured at the end of the session): inter-agent messaging
(`mecha-core` grew with the mailbox store,
taint-forwarding, and the review's fix tests), the benchmark-diagnosis fixes
(overflow-recovery, empty-turn, and session-rewrite regression tests, including
the review-caught rewrite-drops-stale-taint-positions one), the Slack
transport with its binding store and thread state machine, and the outbox
review fixes that came with the factory's wider tool surface. One flake was seen
once in `mecha-core` on 2026-08-08 and never reproduced across five re-runs —
unidentified, worth an eye.

| Suite | Count |
|---|---:|
| `mecha-core` unit | 396 |
| `mecha-cli` unit | 143 |
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
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep, and `slack` for the remote control |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning. Review lives here too: `/outbox` and `/frontdoor` modals drive the CLI like `/triggers` does, the status line badges pending drafts, and `/review now\|later\|auto` decides what happens when a run stages some — scoped to that run's items by an id-diff, tainted drafts never auto-released, the mode set only by command (never parsed from the prompt). Detached releases/extractions/triages are watched and their results reported without a reopen |
| Slack | `mecha slack` — a remote control: Socket Mode from home, an owner allowlist bound by a locally printed nonce, a thread as a `Conversation`, streamed answers with a task card per tool call, approval cards (incl. "allow for this run"), outbox review cards, files both ways, `notify`. **Merged 2026-08-09 (PR #25) and running as `mecha-slack.service`** |
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
| 8080 | Qwen3.6-35B-A3B | up, `total_slots=1`, **`-c 262144`** — the model's whole trained window (`qwen35moe.context_length`), raised from 32768 on 2026-08-10 after re-measuring. **`-c` costs nothing in speed**: 32k/64k/128k/256k are within noise of each other (~92 tok/s at a 1k prompt, ~80 at 30k), and the 50x slowdown recorded on 2026-08-07 was that day's OOM, not the flag. It costs memory as a startup *reservation* — 21.4 GB at 32k to 28.5 GB at 256k, i.e. weights ~20.7 GB plus ~32 KiB/token. **The full tables, the needle test at 188k, the `-np` trade-off and the two traps live in `scripts/start-moe-mtp.sh`** — read it before touching any of this. **`--reasoning-budget 4096`** (2026-08-07) was believed to be the mitigation for this model's "non-terminating reasoning" — **that diagnosis was wrong and is retired as of 2026-08-10 evening**: the empty turns were tool calls emitted before `</think>` closed, one of them 120 characters long, so no token budget was ever involved. The flag is harmless and stays; the real cause and fix are in `CHANGELOG.md` under 0.1.2. The nudge-retry allowance still resets on productive turns, which remains correct for its own reasons. `~/.mecha/config.toml` and `bench/mecha_agent.py` carry `context_window` (= `-c`) and `max_tokens` (**above** the budget; 8192) — four numbers that move together. `-np 1` means every fan-out serializes. MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`). **A transient unit** (`systemctl --user status llama-qwen`), not a tmux pane — see below |
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

**Publishing a release changes nothing about a running system.** The installed
binary is `~/.cargo/bin/mecha`, from crates.io, and every long-lived unit keeps
whatever it started with. After tagging v0.1.2 the box was still executing
0.1.1 everywhere — including `mecha-triggers`, which would have fired that
night's scheduled runs on the pre-fix harness, and `mecha-ruminate` at 03:30,
which mines sessions into learned rules that ride in every future prompt.
Use the three-crate line below rather than upgrading `mecha-cli` alone —
`mecha-mail` ships the MCP server binaries and a run spawns them fresh from
whatever is installed, so a partial upgrade leaves the mail surface a version
behind with nothing to say so. Done 2026-08-10 evening
(`cargo install mecha-cli mecha-mail --locked`, both at 0.1.2), then
`systemctl --user restart mecha-slack mecha-drain mecha-triggers` — verified,
and the connector reconnected with its 12 threads intact at 14:28. The timer-driven units — `mecha-slots`, `mecha-frontdoor`,
`mecha-ruminate` — are oneshot and exec a fresh `mecha` each fire, so they need
no restart; restarting them would be cargo-culting. The set that needs action
is exactly the set holding a long-lived process.

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
  the timer's script was checked against it before enabling.

  **Since 2026-08-10 the installs come from crates.io, not from the repo**,
  which is the better habit now that releases are tagged: it installs exactly
  what the world gets, so a broken package is found here rather than by a
  stranger.

  ```bash
  systemctl --user stop mecha-slack mecha-drain mecha-triggers
  cargo install mecha-cli mecha-mail mecha-factory-publish --locked
  systemctl --user start mecha-slack mecha-drain mecha-triggers
  ```

  Currently installed (2026-08-10 evening): `mecha-cli` 0.1.2, `mecha-mail` 0.1.2,
  `mecha-factory-publish` 0.2.1. `mecha.prev` / `factory-publish.prev` remain
  as the pre-crates.io rollback copies.

  Three things learned doing it, none specific to this change:

  - **`cargo install` can resolve a version older than the one just
    published**, because the local registry index is cached — it fetched
    `mecha-factory-publish` 0.2.0 and `mecha-mail` 0.1.0 minutes after 0.2.1
    and 0.1.1 went up. Check `cargo install --list` against the intended
    versions afterwards, or pass `--version` explicitly. Nothing warns.
  - **Stop the services first.** `cp` over a running binary fails with `Text
    file busy`, and two consumers hold `factory-publish` at all times:
    `mecha-drain.service`'s long-poll loop, and the MCP server the Slack
    connector spawned. (`cargo install` renames rather than copies, so it
    usually survives, but the mail binaries are held by whatever MCP servers
    are alive.)
  - **A long-lived MCP server keeps the tool surface it started with.**
    `mecha-slack.service` served the old seven-tool factory surface until it
    was restarted; a fresh `mecha chat` / `tui` / `run` spawns a new server and
    sees all fifteen immediately. It has now been restarted, though
    `[slack] tools` was emptied the same day, so a Slack thread now carries the
    same surface as `chat` and `tui` — including `mail__*` and `pkg`.
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

Every item below was verified against source on 2026-08-10 night to still be
unbuilt — MCP resources and HTTP/SSE transports, the subagent workspace field,
per-command approval, the Landlock backend, `Rule`'s missing scope, the raw
reflection window, a task store, file watchers and a TUI export are each still
absent from the file the item names. One item was struck the same pass:
`decode_usage` now reads `prompt_tokens_details.cached_tokens`
(`mecha-core/src/provider/openai.rs:309`), shipped with the reasoning arc, and
has moved to [`HISTORY.md`](HISTORY.md).
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
- **A poll cannot hold a real picture, because there is no asset endpoint on
  the box.** Questions and options take `media = { src, alt }` as of
  2026-08-10, and the two sources that render are the only two the policy
  allows: a `data:` URI, or a path this origin already serves. Every class
  sends `img-src 'self' data:`, so the obvious idea — publish a bundle of
  figures and point the poll at it — is refused, and correctly: the artifact
  host is a *different origin* and the browser would block it. An off-origin
  `src` fails when the spec is parsed rather than becoming a hole in a page
  sixty people are looking at.

  What that leaves is inline images capped at 512 KB before base64, which is a
  generous diagram and a hopeless photograph. So "poll a set of images" works
  today only for figures somebody exported small on purpose, and the cap
  cannot simply be raised: a spec travels as one request body through
  `poll_create` and is stored whole, so the ceiling is protecting the store
  and the request, not the page.

  The fix is an **asset endpoint on the box**, scoped to a poll and served
  same-origin, with the questions worth deciding before any code: who may
  upload (the `slots` key, presumably, since that is what creates polls),
  what the per-poll and per-file caps are, which content types are allowed
  and whether the box re-derives the type rather than believing the
  `Content-Type`, and what removes the bytes when a poll is deleted — a poll
  is the one object here with no retention story of its own. Until then
  `polls.md` says the limitation out loud rather than letting a reader
  discover it at the worst moment.
- **Cosmetic:** `factory-publish type push` prints a `/f/<handle>/<id>` URL
  for booking manifests; a booking's page is `/s/…`.

### Cheap, and worth doing first

- **Decide whether replayed reasoning stays unbounded.** As of 0.1.2 the
  OpenAI-compatible backend sends every `Block::Thinking` back, which is what
  the model's own template expects and what took the empty turns from 6/6 to
  0/6. The cost is bounded in *compute* — the prefix cache absorbs it, measured
  at better than 95% reuse — but not in *context*: on the 08-10 run the model
  averaged ~930 output tokens a turn, most of it reasoning, so an 80-turn trial
  carries roughly 75k extra tokens by the end. Against a 174,762 threshold that
  is survivable and it may bring compaction forward on long trials, trading a
  cheap failure (a wasted nudge turn) for an expensive one (a lossy summary).
  **The measurement that decides it is `expect.min_compactions` and the
  compaction counts in the relaunched benchmark's transcripts**; if those jump
  on the long trials, bound preservation to the last N assistant turns. Left
  unbounded on purpose until there is a number: it is the behaviour the model
  was trained with, and guessing a bound first would be tuning against nothing.
  Note this only became affordable at 262,144 — at the old 32k it would have
  been fatal, so the window raise and this fix are coupled.

- **Decide what the compaction threshold should be, now that it moved 8x on
  its own.** `AgentConfig::compact_at` derives two thirds of the window when
  `[agent] compact_at_tokens` is unset, so raising `-c` from 32768 to 262144
  took the threshold from 21,845 to **174,762** as a side effect nobody chose.
  Nothing is broken by it — prompt caching means a growing transcript is only
  prefilled at the delta — but two things argue for setting it explicitly and
  lower. A cache *miss* at that depth costs ~120s of prefill before the first
  token, and a model's useful context is generally shorter than its trained
  one, so a transcript allowed to reach 174k may be answered worse than one
  compacted at 100k. Left unset deliberately: it is a judgement about how runs
  should feel, not a fact that can be measured, and the reader who decides it
  should know it is currently a default rather than a decision.

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
  **github.com/ljchang/mecha-factory** (384 tests, re-measured 2026-08-10),
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

  **An artifact now has two URLs and the tools say which is which
  (2026-08-10 night, merged, unreleased).** A publish reports the gate's
  viewer page beside the bare artifact URL; the account page and the docs
  point at the page; visibility is `Option` end to end so a caller asserts one
  only when told one. The arc, its three fallout bugs and the review that
  caught the worst of them are in [`HISTORY.md`](HISTORY.md). Two things a
  reader needs from it here:

  - **It is on `main` and not deployed.** Until the box runs a build carrying
    `viewer_url`, `factory-publish` prints one column and the MCP answer names
    one URL — the designed degradation, verified live against the current
    origin. `mecha-manifest`/`mecha-factory`/`mecha-factory-publish` at the
    next tag, then `factory-deploy`, is what turns it on. The version bump was
    deliberately **not** made: 0.2.2 is the intended number (the public-API
    break is knowingly shipped as a patch — crates.io reports zero reverse
    dependencies), and the tag is being held to carry other work with it.
  - **The local store and the box can now disagree about visibility**, and
    nothing reconciles them. `bundle_publish`/`bundle_alias` still derive a
    concrete visibility for the local record — its schema has no absence —
    while sending `None` to the box. That is correct as written and is why
    `factory-publish list` can report a visibility the origin does not hold.
    Closing it means either reading the box's answer back onto the record or
    dropping visibility from the local store; it was left out of a URL change
    on purpose.

  **Open there, in the order they bite:**

  - **The MCP surface tracks the CLI again, and two small things are left.**
    The drift is closed — fifteen tools, `surface::REACH` making every command
    exposed-or-excluded in writing, and the arc is in
    [`HISTORY.md`](HISTORY.md). What was deliberately left:

    - **`poll_status` reads its question prompts back from the box**, and now
      the answers too. The prose is the point of a text question, and
      `openWorldHint` marks the whole result untrusted, which is the mechanism
      mecha already uses for mail bodies — so what is left is the user's *own*
      question text round-tripped through an origin the design assumes is
      lost. Smaller than it sounds, and recorded rather than fixed; closing it
      means caching the spec in the local record at create time and rendering
      prompts from home.
    - **`vendor_runtime` is the notebook tool's one reach for the network**,
      fetching Pyodide from a pinned allowlist. It is *not* code execution —
      that belief was measured false on 2026-08-10 (see
      [`HISTORY.md`](HISTORY.md)) — so a confined renderer would need network
      for this template, not an execution exemption.

    Unchanged and worth re-reading before adding more: every exposed tool is
    schema tokens in every run, measured at ~7–8k a turn before this grew the
    surface by eight. That argues for narrowing per surface (`[slack] tools`,
    `[tools] enabled`) rather than for exposing less.

    **The box runs v0.2.1 as of 2026-08-10**, so the poll UI work is live:
    `factory-deploy v0.2.1` downloaded, checksummed, proved the binary and the
    config while the old one was still serving, swapped, and health-checked —
    the whole procedure exercised end to end for the first time on a real
    release. The served stylesheet went from 23,397 to 30,939 bytes and now
    carries the rank counters, the card-select rules and the VAS anchors.
    Worth remembering that `poll_render.rs` lives in `mecha-manifest`, which
    the *box* links: a rendering change is a box deploy, not a home reinstall.

  - **The release workflow is verified** — `v0.2.1` (2026-08-10)
    built the static musl `factory`, asserted it, checksummed it, attached
    both to the GitHub release, and published `mecha-manifest`,
    `mecha-factory` and `mecha-factory-publish` at 0.2.1. The download-and-
    verify procedure in `DEPLOY.md` now has a real artifact behind it.
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
  The arc merged on 2026-08-09 (**PR #25**) and is described in
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
  exists yet**, and there have now been three incomplete attempts. The
  2026-08-07 05:22 launch was voided by the glibc trap; the 11:18 relaunch was
  stopped by hand ~4h in; and the **2026-08-10 03:39 launch at the 262k
  window** (`max_turns` 80, `--agent-timeout-multiplier 2.0`, k=1, preflighted
  with `check-subset.py`) was **killed deliberately at 28/75 that evening**,
  because the reasoning round-trip bug found mid-run meant it was measuring a
  defect that was by then fixed. Its numbers, for whatever they are worth as a
  before: 13 passes of 28 (46%), five `AgentTimeoutError`, one trial lost to
  the dash-prompt crash, ~930 output tokens per turn. Do not treat it as a
  baseline — three known defects are baked into it (stripped reasoning causing
  empty turns, the argv crash, and a timeout tail), and the reasoning fix
  changes per-turn context. Its artifacts are kept at
  `.claude/worktrees/bench-run-262k/jobs/` — that checkout exists only to hold
  them, because those transcripts are the evidence the 0.1.2 diagnosis rests
  on and benchmark artifacts are gitignored.
  **The relaunch is `mecha-arm64-subset-2026-08-10__14-15-05`**, launched
  14:15 from `.claude/worktrees/bench-run-v012` (detached at `v0.1.2`), same
  parameters, `check-subset.py` green on exactly the 75. Judge it on four
  falsifiable things rather than on the score, none of which need the old run
  as a baseline: empty-turn nudges should approach zero (`grep "turn produced
  no content"` across the trials' `stderr.log`), the dash-prompt crash should
  not recur, the timeout tail should shrink from 5-in-28, and **compaction
  counts should not jump** — that last one is what settles whether replayed
  reasoning stays unbounded. The 2026-08-07 fragment was separately
  **diagnosed trial by trial** (2026-08-09 — the write-up is in
  `docs/BENCHMARK-RESEARCH.md`, "The 2026-08-07 subset run, diagnosed"): of
  13 failures, 5 were the model and 8 involved harness defects, all five of
  which are fixed with regression tests (PR #21 — overflow-recovery
  poisoning, cumulative empty-turn allowance, exit-3-on-exhausted read as an
  agent crash, transcripts lost on crash or corrupted by compaction, and a
  flat output budget bigger than the threshold-to-window gap). Every one of
  those trials also ran against a 32k window that never needed to be 32k
  (2026-08-10 — see "Environment as left"), so compaction pressure was a live
  variable there and is largely gone at 262,144: the fragment is not a
  baseline for anything current either.
  **Budget the wall clock honestly.** The ~15h figure quoted here for months
  was never measured; the 08-10 run's own trials averaged 34 minutes and
  projected **~29h at k=1**, because `--n-concurrent-agents 1` serialises every
  agent phase against the single `-np 1` slot. Timeouts dominate the tail — a
  runaway spends the full multiplied cap (2.0x meant two hours for one trial).
  If that is too long, `-np 4` with `-c` raised to match is the measured
  trade (12% of single-stream speed for 1.6x aggregate; see
  `scripts/start-moe-mtp.sh`), and it needs `context_window` in
  `bench/mecha_agent.py` moved to the per-slot figure in the same change.
  Rebuild the portable
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
