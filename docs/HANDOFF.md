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

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.9**
(2026-08-20; 0.1.7 and 0.1.8 shipped 2026-08-19 and 2026-08-20 after the mail
hold lifted). **Four** crates are on crates.io — `mecha-core`, `mecha-mail`,
`mecha-slack`, `mecha-cli` (the bare name `mecha` was taken, so the CLI
crate installs the `mecha` binary) — published through the tag-driven
`release` workflow with Trusted Publishing, so no registry token exists
anywhere. From v0.1.1 on, a tag and a crate version are the same number.

The knowledge graph is now a **sibling public project**:
**github.com/ljchang/mecha-graph** (fresh history — the private repo's
journal is the data the project exists to hold, so the public tree starts
where every fixture wears a synthetic world), with `mecha-graph`,
`mecha-graph-core` and `mecha-graph-mcp` on crates.io at 0.1.0, hand-published
2026-08-16 (no release workflow there yet; mecha's is the template). The CLI
crate took the bare name deliberately — `cargo install mecha-graph` is the
front door, and mecha-cli's own name was necessity, not preference. mecha
wires it as `[[mcp]] name = "graph"` with `prefix_tools = false`, so the
model calls bare `kg_*` names; the store lives at `~/.mecha-graph/` under
`MECHA_GRAPH_*` env vars.

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
owns the drift check, because that is where the renderer lives. `overview.md`
landed 2026-08-10; five more pages are planned in
[`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md).

Every commit was verified to build and pass tests **in isolation**, so the
history bisects rather than merely ending in a good state.

First thing to run in a fresh context:

```bash
cargo test --workspace && cargo clippy --all-targets --all-features
```

Expect **1,140 tests**, no failures — re-measured 2026-08-20 on `main` at
`cfa2cc2` (629 in the `mecha-core` lib suite, 297 in `mecha-cli`, 122 in
`mecha-mail`, 75 in `mecha-slack`, and 17 across the two integration suites
that need real backends). The earlier 2026-08-20 count was 1,105 at 0.1.9,
the 2026-08-19 count was 989 and the 2026-08-18 one 936; the growth from 707 (2026-08-10) spans the 0.1.3–0.1.9 arcs, and each
release's CHANGELOG entry names what its tests pin. **A flake has now been seen twice and is still unidentified.** Once in
`mecha-core` on 2026-08-08, and again on 2026-08-19 (`cargo test --workspace`
reported `506 passed; 1 failed` in the `mecha-core` lib suite, then four
consecutive clean runs of the same suite and the whole workspace). The failing
test's *name* was not captured either time, which is the thing to fix next
sighting: run the suite so the `failures:` block survives, rather than grepping
for a summary line. Two sightings eleven days apart in the same crate is a
pattern forming, not noise to keep ignoring — the likeliest suspects are the
tests that share process-global state (`MECHA_HOME`, which the trigger and work
tests set behind a lock) or the `temp_store` helpers keyed on pid plus thread
id.

| Suite | Count |
|---|---:|
| `mecha-core` unit | 472 |
| `mecha-cli` unit | 223 |
| `mecha-mail` unit | 101 |
| `mecha-slack` unit | 75 |
| integration (`mcp_server` 6 + `sandbox_backends` 9) | 15 |
| doctest | 2 |

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
| Tools | `fs_read/write/edit/list`, `shell`, `http_fetch`, `todo`, `web_search`, `ask_user`; `recall` on session-recording front-ends (chat, TUI, resumed runs) — searches the transcript union, including what compaction rewrote away |
| Planning | `Phase::Plan` hides writing tools structurally — not offered, not dispatchable |
| MCP | stdio client; per-server on/off; capability overrides that only widen; `prefix_tools = false` for a server whose tools carry their own namespace (collision fails startup loudly) |
| Memory | mecha-graph (the artist formerly known as pkg, now public) wired as MCP server `graph`, unprefixed `kg_*` tools — the user's mail, Slack, calendar, conversations |
| Subagents | `Agent` wrapped as a `Tool`, allowlisted registry, per-profile model |
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — with fall-through |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Sandbox | `shell` and MCP servers confined via bubblewrap, docker, or landlock (no-privilege file confinement; never narrows `external_send` — UDP is unrestrictable, and the preflight plants a home file and requires the confined read to fail) |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Context | Two-pass compaction: thin tool results, then summarise. Taint preserved, a tool's own state (the todo list) crosses verbatim, the states a mid-run rewrite replaced ride `Conversation::rewritten` into the session record, and a per-run cache lens watches whether the cached prefix is actually reused (warns only on unexplained re-payment) |
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep, and `slack` for the remote control |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning. Review lives here too: `/outbox`, `/frontdoor`, `/mail`, `/tasks`, `/skills`, `/polls` and `/doctor` modals drive the CLI like `/triggers` does, the status line badges pending drafts, and `/review now\|later\|auto` decides what happens when a run stages some — scoped to that run's items by an id-diff, tainted drafts never auto-released, the mode set only by command (never parsed from the prompt). Detached releases/extractions/triages are watched and their results reported without a reopen |
| Slack | `mecha slack` — a remote control: Socket Mode from home, an owner allowlist bound by a locally printed nonce, a thread as a `Conversation`, streamed answers with a task card per tool call, approval cards (incl. "allow for this run"), outbox review cards, files both ways, `notify`. **Merged 2026-08-09 (PR #25) and running as `mecha-slack.service`** |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit/**review**/send/reject, several ids or `--all` narrowed by `--kind`/`--via`; edits mined as writing reflections. Items carry a kind — a publish shows its rendered page, refuses `edit`, and is excluded from the miner — and the jail they were drafted under, so a release resolves paths against the agent's workspace rather than the reviewer's |
| Messaging | `[messages]` + `mecha msg send/list/show/dismiss/agents` — a file mailbox between this machine's sessions (`~/.mecha/messages/<recipient>/`, producer-name addressing, per-session liveness registry). Delivery folds in at the steering point with the sender's harness-stamped taint merged first, so a hop launders nothing; attended surfaces hold with a notice, unattended accept; global config only; full mailboxes refuse, identical pending sends dedup. `docs/MESSAGING-RESEARCH.md` is the design record; phase 2 (TUI modal/badge) is scoped there |
| Workspaces | `~/.mecha/work/<producer>/` is a run's workspace and its output directory; `mecha work list/path/clean`, retention nightly. A workspace containing the mecha home is refused |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed. **`mail_triage`** (2026-08-18) adds archive/read/unread/spam/trash as a closed `TriageAction` enum, thread-level, in a third capability quadrant — `destructive` but *not* `external_send`, so it never routes through the outbox and a read-only run cannot reach it. Tagging is deliberately absent: a tag is mecha's own, on the triage record, not a Gmail label or a Graph category |
| Tasks | `mecha tasks` list/add/set and the `/tasks` modal onto the graph's GTD board, reached only over `kg_task_*` — no dependency on mecha-graph and no second reader of its schema. Status letters match `mecha-graph tui` screen 6; nothing confirms (the board reaches nobody and has no delete); a reload re-finds the cursor by id because a status change reorders the board |
| Documents | `mecha-docs`, the fourth binary on `mecha-mail` — Google Docs/Sheets/Slides under **`drive.file` and nothing else**, so only files mecha created or the user picked in Google's own chooser are reachable, and no instruction inside a run can widen that. Reads are `untrusted_input` and never `openWorldHint`; writes are outbox-routed, because writing into a document a third party can read is a publish. No permanent-delete and no sharing verb, with tests on the absences |
| Front door | `mecha frontdoor` list/show/extract/next/**triage**/**needs-info**/**close** over `~/.mecha/requests/` — the quarantine between a stranger's request and a run with tools, and the state machine that lets one reach an answer. The extractor is issued no tools and no history; `Record::for_privileged_run` has no argument that returns the prose; an extraction failure routes to a human. `triage` drafts into the outbox and refuses to run unrouted; `reconcile` closes the loop from released items on its own, with no verb to remember. `mecha-factory-publish drain` fills the directory |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, **the daemon is installed and running here**; a failed `notify` is recorded on the run |
| Skills | `~/.mecha/skills/<name>/SKILL.md` in the Agent Skills format, loaded by a `skill` tool call at three levels of disclosure. User-authored with no mechanism for anything else — no install, no registry, no remote body, none derived from a session — which is why loading one arms no taint. `tools:` narrows the surface and can never widen it; a loaded skill crosses compaction verbatim; `mecha eval` forces them off |
| Learning | the full arc: reflect-on-close → nightly rumination → counterfactual validation (steers/denials trace-graded) → gated proposals (`mecha proposals`); git-backed store under `~/.mecha/learning`; rules carry id/sources/created_at, validate feeds a per-rule outcome ledger with regression bisection, and `mecha rules` retires through the same gate (`eval --ab-rules` for the coarse A/B). Budget is 25 active rules and 2600 chars **per domain**, and a run carries only `RUN_DOMAINS` (`behavior` + `writing`) — new domains are opt-in and `unrouted_domains` warns at startup on any that ride in no prompt |
| Run quality | `Record::Outcome(RunStats)` per finished run from every front-end; `runlog.rs` reads the corpus back (`mecha sessions health`, rates split by model, `—` where a denominator is zero); three population checks in `doctor`; `candidate.rs` gates a proposed change on a paired comparison with a deterministic holdout and a work guardrail; `mecha eval --ab-config KEY=VALUE` is the content-sensitive arm; `mecha diagnose` proposes one change from the corpus and prints the command that would falsify it. **Nothing is applied automatically and the corpus was empty at build time** — see below |
| Eval | 36 cases, 15 tags (both re-counted 2026-08-20, unchanged), scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `graph-cases.jsonl` — 10 memory/interlock cases against fixture MCP servers (`--mcp-file`), renamed with the graph and expecting the bare `kg_*` names production serves (scorecards across the rename are not comparable) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

> **This checkout is a live service's `ExecStart`. Do not `git stash`, `git
> checkout --` or `git restore` `scripts/start-moe-mtp.sh`.**
> `~/.config/systemd/user/llama-local.service` names
> `/home/ljchang/Github/mecha/scripts/start-moe-mtp.sh` literally, and the unit
> is active (verified 2026-08-20, started 15:30 UTC). The working copy is
> parameterised — `NP=4`, `CTX=1048576`, `--cache-idle-slots` deliberately
> absent with a "do not add it back" note — and is **uncommitted**. `HEAD`'s
> copy hardcodes `-c 262144 -np 1` and re-adds `--cache-idle-slots`, so any
> reflex that reverts a dirty file silently rewrites the server's launch
> command; it comes back healthy on the next restart and just behaves
> differently. Precisely: the per-slot window is 262144 either way, so
> `context_window` stays correct — what is lost is the four slots (every
> fan-out serializes again) and the deliberate absence of a flag. The wider
> lesson is in HISTORY under Environment: **"move it to a worktree" assumes
> nothing outside the repository points at a path inside it**, and here systemd
> does, so relocating would break the running server rather than protect it.
> Three arcs shared this checkout on 2026-08-20 and all three staged by path.

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Re-verified
2026-08-20** (8080 answering with `total_slots=1`; 8081, 8082 **and 8083** all
down; SearXNG up). **The installed binaries are at 0.1.8 and the repository is
at 0.1.9** — `~/.cargo/bin/mecha` was last built 2026-08-20 00:26, before the
four arcs of that night, so nothing in this release is in the binary the
machine actually runs until the `update` skill is run.

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, `total_slots=1`, **`-c 262144`** — the model's whole trained window (`qwen35moe.context_length`), raised from 32768 on 2026-08-10 after re-measuring. **`-c` costs nothing in speed**: 32k/64k/128k/256k are within noise of each other (~92 tok/s at a 1k prompt, ~80 at 30k), and the 50x slowdown recorded on 2026-08-07 was that day's OOM, not the flag. It costs memory as a startup *reservation* — 21.4 GB at 32k to 28.5 GB at 256k, i.e. weights ~20.7 GB plus ~32 KiB/token. **The full tables, the needle test at 188k, the `-np` trade-off and the two traps live in `scripts/start-moe-mtp.sh`** — read it before touching any of this. **`--reasoning-budget 4096`** (2026-08-07) was believed to be the mitigation for this model's "non-terminating reasoning" — **that diagnosis was wrong and is retired as of 2026-08-10 evening**: the empty turns were tool calls emitted before `</think>` closed, one of them 120 characters long, so no token budget was ever involved. The flag is harmless and stays; the real cause and fix are in `CHANGELOG.md` under 0.1.2. The nudge-retry allowance still resets on productive turns, which remains correct for its own reasons. `~/.mecha/config.toml` and `bench/mecha_agent.py` carry `context_window` and `max_tokens` (**above** the budget; 8192) — four numbers that move together. **`context_window` is `-c / -np`, not `-c`** — llama-server divides the context across slots, so the rule this line used to state was right only by accident of `-np 1`. Read it off `/props` (`default_generation_settings.n_ctx`) or the startup line's `n_ctx_slot`, never by arithmetic on the flag. **This row's numbers are stale and a live arc is rewriting them** — see the open item under *What to do next*. MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`). **A transient unit** — now `llama-local.service` (`systemctl --user status llama-local`; it was `llama-qwen` when this was written, and that name no longer resolves), not a tmux pane — see below |
| 8081 | gemma-4-E4B | down; nothing currently depends on it |
| 8083 | Qwen3.8-27B | **down as of 2026-08-20** (was up on 2026-08-16). Nothing in config depends on it, so nothing is broken by it — noted because the previous pass recorded it up and a reader would otherwise assume it still is |
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

**A release key sits on an agent machine, and that is against the rule the
factory wrote down.** `~/.mecha/factory/release.key` exists here (mode 0600,
2026-08-07), on the same box that runs `mecha-slack.service` and
`mecha-triggers.service` — both active. `mecha-factory/docs/SECOND-CLIENT.md:58`
says "Keep release keys off machines agents use", and the reason is the
ceiling: a paired agent machine's intended worst case is *immutable versions
nobody can read*, and a release key removes that bound. Under mecha the
publish path is outbox-routed so a human still gates each release, which is
mitigation and not the property the rule protects — an unattended run on this
box is one confused-deputy step from a capability the design says it should not
be able to reach. **Surfaced, not acted on**: moving the key is a decision
about how this machine is operated, and it belongs to whoever operates it.
(Verified 2026-08-20.)

**The KV cache is f16, not q8_0 — on either server.** No `-ctk`/`-ctv` on
mecha's llama-server and no `LLAMA_ARG_CACHE_TYPE_*` in its environment; and
`systemctl show ollama.service -p Environment` carries `PATH` and nothing else,
so `OLLAMA_KV_CACHE_TYPE` is unset there too (all verified 2026-08-20). Nothing
in this repository ever claimed otherwise.

It is written down anyway because **the belief has a live source on disk and
will regenerate.** `~/Github/dgx-spark-playbooks/nvidia/txt2kg/README.md:153` —
NVIDIA's own tuning playbook for this hardware — recommends
`OLLAMA_KV_CACHE_TYPE=q8_0` in a *troubleshooting* table ("Ollama performance
issues → Suboptimal settings for DGX Spark"), and `assets/README.md:99` states
"Q8_0 KV cache for memory efficiency" flatly. Both are advice for **ollama**,
not for mecha's llama-server, and neither was ever applied. A recommendation
read as a description is how a machine acquires a property nobody gave it; the
playbook is still there and still says it, so the next reader will believe it
again unless this measurement is beside it.

**Restarting a model server under memory pressure is permanent for that
server's life.** `scripts/start-moe-mtp.sh` records the finding; what it means
operationally is that the `update` sequence must not restart
`llama-local.service` while anything heavy is resident — a second llama-server
under `mecha-graph extract`, say — because a server that loads under contention
stays slow and never recovers. Check tok/s afterwards, not just that the unit
came back up. Liveness is the check that cannot see this failure.

**Two live config changes exist in no repository.** `~/.mecha/` is not a git
repo, so `config.toml` is on one disk and in no history. As of 2026-08-20 it
carries a `[[mcp]] name = "docs"` block running `~/.cargo/bin/mecha-docs`, and
six `docs__*` names in `[outbox] tools` (`docs_create`, `docs_append`,
`docs_replace`, `sheets_create`, `sheets_write`, `slides_create`) — verified
live on the surface after the 0.1.9 install: writes staged, reads read-only,
and `docs_trash` deliberately unrouted because it reaches nobody. A fresh clone
plus a fresh `~/.mecha` would have the binary and none of this wiring.

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

- **Two live changes on 2026-08-19 that exist in no repository**, both under
  `~/.mecha/`, which is not a git checkout. The `morning` trigger's `notify`
  line now appends day two to the briefing by shelling out to
  `mecha mail list --aged --surface`, so **the installed binary must carry
  `--aged` before 07:00 or the briefing pastes a clap usage error into
  itself** — caught on the day by running the hook rather than reading it. And
  `llama-local.service` is new (below). A fresh clone has neither.
- **The local model server is `llama-local.service`** (systemd user, enabled,
  `scripts/start-moe-mtp.sh`, qwen3.6-35b-a3b on 127.0.0.1:8080). **It became a
  unit on 2026-08-19 and the reason generalises.** Before that it was only ever
  started as a transient unit, so a reboot restored every *consumer* —
  `mecha-triggers`, `mecha-slack`, `mecha-drain`, `mecha-mail-classify` are all
  persistent and enabled — and none of the thing they consume. The machine
  rebooted at 02:39 that morning and for nine hours every agent run failed with
  a connection error while systemd reported a healthy system. A reboot does not
  degrade this box evenly: it brings back exactly the half that generates load.
  `Restart=on-failure`, deliberately not `always` — a server that cannot load
  should stay visibly down rather than loop.
  **After restarting it, measure tokens/sec and not merely that it answered**:
  `start-moe-mtp.sh` records that a server which loads while memory is
  contended stays slow for its whole life and never recovers. 100.5 tok/s on a
  short prompt is a healthy load; ~82 is the degraded one.
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
    same surface as `chat` and `tui` — including `mail__*` and the graph.
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

**Mail OAuth grants, re-consented 2026-08-18** (both were re-issued that day
because the triage scopes widened, and both are recorded in each account's
`oauth.json` under `granted_scopes`):

| Account | Provider | Grant | Expiry |
|---|---|---|---|
| `personal` | Google | `gmail.modify`, `gmail.send`, `calendar`, `calendar.events` | **7 days from consent — next ≈2026-08-25** |
| `dartmouth` | Outlook | `Mail.ReadWrite`, `Mail.Read`, `Mail.Send`, `Calendars.ReadWrite` | none — permanent |

The Google client (Cloud project **FlowMail**, the same registration the old
app used) is in **Testing** publishing status with User type External, and
Google expires a Testing app's refresh token exactly 7 days after consent —
refreshing does not extend it. Moving to production would fix that but needs
verification plus a CASA security assessment (~$540/yr), because
`gmail.modify` is a restricted scope. **Decided 2026-08-18: stay in Testing,
and revisit CASA once the main development features are done** — so this is
deferred rather than open, and should not be re-litigated as though it were
undecided.

Two things to carry into that revisit, both measured on 2026-08-18 by the
parallel documents work (`docs/DOCS-RESEARCH.md` §6.2): the console
distinguishes **brand verification** from **scope verification** and only the
second is the expensive one — a banner reading "your app requires
verification" appeared on a project with no sensitive or restricted scopes at
all and turned out to be branding, which blocks nothing and must not be
answered by submitting for review. Check the Verification Center's two cards,
never the banner. And a Google grant is per (user, client), not per scope, so
anything added to the FlowMail client shares mail's fate in both directions —
which is why the documents work took its own project rather than this one.

Meanwhile
`~/.mecha/mail/accounts.toml` declares `grant_lifetime_days = 7` on `personal`
so `mecha doctor` warns two days out; that file is in no git repository, so a
fresh clone will not have it.

Dartmouth's Entra registration (also named FlowMail, client
`bc6a1e19-…`) already had `Mail.ReadWrite` **Delegated** granted tenant-wide,
so no ITC request was needed — the opposite of what was expected.

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

**A second instrument arrived 2026-08-19 and has its own caveat**: a year of
real mail at `~/.mecha/mail-corpus/`, which can grade the mail classifier
offline with no human in the loop, because the corpus records whether a reply
went out. **The ground truth is asymmetric and must be quoted that way** — a
reply proves the thread mattered, so classifying it `ignore` is a countable
error; *no* reply proves nothing at all, since most unanswered mail correctly
needed no answer and some was settled in a meeting. It measures false-`ignore`
and is silent on false-`respond`. A scorecard from it that claims precision is
reading the instrument wrong. `docs/MAIL-CORPUS-RESEARCH.md` §3 has the two
caveats in full — **gitignored, like `OPERATIONS.md`**, because its figures are
one person's mailbox rather than a public fact.

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

Every item below was re-verified against source on **2026-08-20** — MCP
resources (`mecha-core/src/mcp.rs:206` still advertises `"capabilities": {}`),
HTTP/SSE transports, the subagent workspace field, per-command approval, the
seccomp half of the sandbox item, `Rule`'s missing scope, the raw reflection
window, file watchers and a TUI export are each still absent from the file the
item names, and `gossip`/`vet`/`corroborate` are still in `mecha-cli/src/commands/`.
Three items changed that pass. **Skills** and **Google Docs/Sheets/Slides
write access** both said "not built" and are fully shipped
(`mecha-core/src/skill.rs`; `mecha-mail/src/google/docs.rs`,
`google/docs_server.rs` and a 323-line `bin/mecha-docs.rs`), so both moved to
[`HISTORY.md`](HISTORY.md). The **task store** item shipped in part — the
`/tasks` modal and direct capture exist and the store turned out to be the
graph's board rather than a new one under `~/.mecha/` — and has been rewritten
under Triggers to describe only the escalation half that is still missing.
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

### The self-improvement loop — built, and waiting on its own data

Every stage exists as tested code and **nothing acts on the numbers**. The
open item is not code, it is evidence: run outcomes are recorded from this
version on, so at build time `mecha sessions health` read 178 sessions and
found 0 outcomes. `docs/SELF-IMPROVEMENT-RESEARCH.md` is the authority and §13
records decisions already made — do not re-ask them.

The reason to wait rather than to finish it is in that document's §2: agents
measurably update their harnesses without benefiting, and the fourth named
failure mode is optimizing for update *frequency*. Building the autonomous
driver before knowing whether these findings are worth acting on is that
failure mode by name.

**Two clocks start at install, not at the date this was written**, and the
shared trap is that both features look finished and measure nothing. Anyone
reading `0 rows` a week from now will reasonably assume something broke.

- **The run-quality corpus.** Triggers, the Slack service and the nightly all
  run release paths, so no build that writes `Record::Outcome` is in service
  until the deploy and every run before it records nothing.
- **The mail correction ledger**, per the parallel mail arc, for the same
  reason plus a second: the loop needs *corrections*, and the button that
  produces one (`mecha mail correct`) is likewise only in an uninstalled
  build. Its `mecha mail score` says so in words rather than printing an empty
  table.

As of 2026-08-19 the 0.1.7 release **and any install** are held for mail phase
6 (~25% done at that point), chosen deliberately over an install-now-tag-later
option. So an empty corpus and an empty correction ledger are the expected
readings for the length of the hold. The checks below begin at install.

**Check as data accumulates, in this order.** Each step is cheap and each one
can retire the next:

1. `mecha sessions health --days 30` — does the corpus say anything yet? Is the
   per-model split meaningful, or is everything one model?
2. `mecha doctor` — do the three run-quality checks fire? When one does, the
   question is not whether the number is right but **whether you would have
   acted on it**. A finding you would ignore is a threshold set wrong, not a
   problem you have; the thresholds are deliberately high
   (`ENDED_ON_FAILURE_RATE`, `TOOL_ERROR_RATE`, `CUT_SHORT_RATE` in
   `doctor.rs`) because rule-based evaluators are measured to over-flag.
3. `mecha diagnose --dry-run` — is the brief enough to diagnose from, or is it
   missing a signal that is not being recorded? That answer is worth more than
   any change the diagnostician would propose, because it is a gap in the
   sensor rather than in the model.
4. Only then `mecha diagnose` for real, and `mecha eval --ab-config` on
   whatever it proposes.

What is genuinely unbuilt, and deliberately so until step 3 answers:

- **No nightly stage.** `scripts/ruminate.sh` does not run any of this. It
  belongs there eventually, after `validate`, on the same "a skipped night is
  not a failed night" contract — but a nightly that proposes changes nobody
  reads is worse than no nightly.
- **The arms run over eval cases, not over replayed sessions.**
  `eval --ab-config` is the content-sensitive arm and it is the one that
  exists. `replay_run::drive` now returns `RunStats`
  (`mecha-core/src/replay_run.rs`), so the pieces for a session-corpus arm are
  present and unassembled. Know the limit before building it: replay holds tool
  results fixed, so it can grade a compaction threshold or a rule but is blind
  by construction to `output_budget_bytes`, sandbox, retries and failover —
  §8 of the research has the table.
- **No auto-accept path.** `candidate::Disposition::Accept` is computed and
  nothing consumes it. Wiring it means deciding where the applied change is
  written and how it is reverted, neither of which exists.
- **Security boundaries are gated, not excluded.** Luke's ruling (§13.2) makes
  interlock, path jail, sandbox and outbox routing human-gated like any other
  architecture change. The recommendation on record is that they stay
  unproposable: a loop that can argue for widening its own confinement will
  eventually argue well, and the metric agrees with it, because a run that can
  reach the network fails fewer calls.

### Cheap, and worth doing first

- **`context_window` is `-c / -np` and four derived numbers trusted the wrong
  rule.** Verified 2026-08-20: `/props` on :8080 reports `total_slots: 4` and
  `default_generation_settings.n_ctx: 262144`, against `-c 1048576` in
  `scripts/start-moe-mtp.sh` — so the per-slot window is a quarter of the flag.
  The Environment row above stated `context_window` (= `-c`) and `-np 1`, which
  was true when written and became wrong the moment slots were added; the
  *current* `~/.mecha/config.toml` value (262144) happens to be right, so
  nothing is broken today and the rule was still wrong. It matters because
  **four things derive from `context_window` and all four degrade silently if
  it is wrong**: `AgentConfig::compact_at` (two thirds of the window),
  `ToolsConfig::resolved_output_budget` (an eighth, clamped), the TUI fuel
  gauge, and overflow recovery's expectations. At `-np 4` a stale `= -c` value
  would have set the compaction threshold four times too high — meaning no
  reactive compaction at all, and every long run discovering the limit as a
  raw 400 instead. CLAUDE.md's Context section still states the `-c` rule too.
  **This is somebody else's live arc**, uncommitted as of `cfa2cc2`
  (`CLAUDE.md`, `scripts/start-moe-mtp.sh`, and a new `docs/LLAMA-SERVER.md`
  that is not yet in git), and it already carries the correction. What is left
  for whoever lands it: refresh the Environment row above, and decide whether
  anything should *check* the relationship rather than restate it — a startup
  warning when `context_window` disagrees with `/props` would end this class,
  the same way `unrouted_domains` warns rather than failing quietly.

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
- **A seccomp layer on the sandbox.** Landlock itself shipped 2026-08-16
  (`Backend::Landlock`, `sandbox.rs` — no-privilege file confinement, hard
  ABI-3 floor, a preflight that plants a home file and requires the confined
  read to fail, and deliberately no `external_send` narrowing because UDP is
  unrestrictable). What the research scoped and remains unbuilt is syscall
  filtering: no `seccomp` anywhere in `mecha-core/src/`. Also note this box
  stays on docker for `shell` anyway — docker's `network = false` earns the
  interlock relaxation that Landlock, honestly, cannot.
- **In-run verification / a convergence primitive.** Nothing in `agent.rs` tests
  a post-condition; there is no runtime "is it done yet". The research's own
  answer is the starting point: it has to be a command's exit code, not a
  model's opinion. `compact_validate` is the only in-run verifier that exists.
  Narrowed 2026-08-19: `RunOutcome::ended_on_failed_call` (`agent.rs`) now names
  the *post-hoc* case — a run that stopped of its own accord with its last call
  failed — which is the silent-failure shape a judge cannot catch. It is a
  report, not a convergence test; the gap above is unchanged.
- **Programmatic tool calling** (a `code` tool that calls other tools from inside
  a program). Two hazards to solve first, both named in the research: taint must
  update *within* a running program, and approval for a program that makes
  thirty calls is not thirty approvals.

### The learning system

The arc is complete and running nightly. What is missing is refinement:

- **The sliding window of recent raw reflections never shipped.** Prompt assembly
  chains user rules then consolidated rules; the third leg — a window of recent
  unconsolidated reflections — was designed and not built.
- **Rules are scoped by domain and by run, but not by tool.** A run now selects
  which domains it carries (`RUN_DOMAINS`, 2026-08-18), so a domain no longer
  rides in every prefix by default. What is still missing is the finer grain:
  `Rule` has no `scope` field, and nothing injects rules into a tool's own
  block.
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

- **Deadline escalation, on top of the task surface that now exists.** The
  `/tasks` modal and `mecha tasks` shipped 2026-08-20 — direct capture, status,
  scheduling — so *direct capture* and *the modal* are done and the store is
  the graph's board, reached over `kg_task_*` rather than built in `~/.mecha/`.
  What that arc did **not** build is the part that turns silence into a state:
  no task is created from an inbound request's SLA, none from a **commitment
  the user made** (extractable from released outbox items, where mecha already
  knows what went out), there is no `Origin` per task so nothing can decide
  which tasks may escalate unattended, and nothing escalates. Recurrence still
  wants `cron.rs`. An unanswered message is still invisible; there is now
  somewhere to hang the state, and nothing hangs it. Design in
  `PUBLIC-SURFACE-DESIGN.md` §3.2–3.3.
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

### Mail as a surface you work — four phases remain, reordered by measurement

**`docs/MAIL-UX-DESIGN.md` is the authority for what is left** and
**`docs/MAIL-CORPUS-RESEARCH.md` (2026-08-19) is the measurement that reordered
it — gitignored, on `OPERATIONS.md`'s split: the lesson is here, the figures
are not**; `MAIL-UX-RESEARCH.md` is the original survey. Where they disagree the
later one wins, and the corpus is the latest. **The release hold is over**:
0.1.7 shipped 2026-08-19 and 0.1.8 and 0.1.9 on 2026-08-20, so the phases below
are ordinary open work rather than a blocker on shipping. What the hold bought
is recorded in `HISTORY.md`; what it was holding for — the phases below — is
still unbuilt, and is now unblocked rather than deferred.

Phases 1-3 shipped 2026-08-18: `mail_triage` (archive/read/unread/spam/trash,
closed enum, both providers), `~/.mecha/mail-triage/` holding one typed verdict
per thread, the quarantined classifier, `mecha mail
classify/list/show/dismiss`, the snippet-first escalation rule, and the
`mecha-mail-classify` timer (05:30 UTC, Dartmouth only, installed and running).

**Front-door routing was the original phase 4 and is deleted** (2026-08-19),
along with `ROUTABLE_TYPES`, `is_routable` and `Proposed::Frontdoor`. Do not
rebuild it: `MAIL-UX-DESIGN.md` §1 has the five reasons, the sharpest being
that the front door's `[verification]` block exists to prove a stranger
controls an email address and an email *arrived from* one. Mail keeps its own
request kinds and gets its own `needs-info`.

#### What the year of mail changed

A year was fetched raw and unclassified (`mecha-mail corpus`, an operator verb
deliberately absent from the MCP surface), then analysed. Three findings, each
of which moved the plan rather than confirming it. **Figures are in the
gitignored doc; the conclusions are stated here so nothing below depends on
having read it.**

- **About half of all threads need no model at all** — `List-Unsubscribe` plus
  a sender-address regex, at a negligible error rate over a year. The header
  alone finds two thirds of it; it catches marketing, which must offer an
  unsubscribe, and misses every institutional and transactional sender, which
  need not. **This supersedes the note previously recorded here that "the
  classifier is a better filter than the `List-Unsubscribe` heuristic and runs
  anyway."** It is a better filter; it should not be the first one.
- **The taxonomy was guesswork and wrong in both directions.** Already fixed in
  code: `student-advising` added (the largest category by a wide margin, and
  absent entirely), `advising` added to `TAGS`, `book` removed (two threads in
  ten months, neither a book request). `finance-admin` was recommended
  by the analysis and deliberately rejected — nothing has to be gathered before
  a receipt is forwarded, so it is the `expense` tag and a `forward` action.
- **The failure is abandonment, not misclassification.** Only a small fraction
  of personally-addressed mail is ever answered, most replies that happen
  happen on day one, and **a thread unanswered after a day is overwhelmingly
  unlikely ever to be answered.** No phase in the old plan operated after day
  one.

What is left, in the order the measurements argue for:

- **Pre-filter (part of phase 4′, unbuilt).** Bulk + sender-pattern ahead of
  the classifier. Halves the nightly's model calls, fully specified, and its
  error rate is measurable against the corpus rather than argued about. The
  smallest item here and the one that makes everything after it cheaper to
  iterate on.
- **Corpus as an offline eval set (new, unbuilt, not yet in the design doc).**
  For every Dartmouth thread we know whether a reply went out, so a thread that
  was answered and is classified `ignore` is a countable error with no human
  grader. **Asymmetric on purpose**: a reply proves the thread mattered, no
  reply proves nothing — so it measures false-`ignore` and is silent on
  false-`respond`. That is the error worth catching. Grading the whole corpus
  is days of local inference; after the pre-filter it is about half that, and a
  stratified sample of a few hundred threads is one overnight run.
- **Phase 4″ — tasks and `needs-info`, native to mail.** `t` carries the
  thread's deadline into `kg_task_create`; `n` parks a thread and names what is
  missing. This is the half of the front-door idea that survived.
- **Phase 4‴ — day two.** A `respond` thread aged past the threshold with no
  outbound message since, surfaced once. Needs no new state. Keys on the bucket
  and never on silence, forgives a thread settled in a meeting, and fires once.
  **Surface decided 2026-08-19: the morning briefing**, because it is already
  read daily and day two is precisely when the user is not looking at mail. A
  trigger cannot ask, so the briefing lists and acting happens elsewhere —
  which makes `mecha mail list --aged` the primitive and the briefing one
  reader of it. Still open: the age itself, likely one working day rather than
  24 hours, and measurable against the corpus rather than guessed.
- **Phase 5 — `/mail`**, a sixth modal on the `/outbox` pattern with a closed
  key set. `r`/`e`/`f` are detached agent runs; `a`/`s`/`t`/`g` are single
  calls. Replies land in `/outbox`, which stays the only approval surface.
  `f` is **forward**, which had no key bound to it before — the
  receipts-to-the-finance-person case, one of the five that motivated the
  feature, had no way to happen.
- **Phase 6 — the correction loop.** `!` records a field-level correction,
  feeding a classifier few-shot pool *and* a `triage`-domain reflection on the
  ordinary learning path. The pool is deliberately not a learned rule —
  `triage` is not in `RUN_DOMAINS`.

Six open questions are in the design doc's §7. Only one blocks: where day two
surfaces. The rest are live but not in the way — whether `r` hands the drafting
run the thread or the verdict; whether `t` can point back at the thread at all
given `kg_task_create`'s fields; whether `meeting` earns its place; retention on
the triage store; and **how `student-advising` is actually answered**, which is
the biggest single piece of the load and is probably a substitution problem —
the same handful of questions, which a form or a published answer removes
rather than something mecha should answer one at a time.

Two decisions recorded rather than left open: tags are mecha's own and never
provider labels, and no mail parser belongs in mecha — the graph already ingests
`email.thread` episodes (`sources/mbox.rs`) with the bulk filter and the
`NEVER_AUTO` guard, so the live path pushes evidence through `kg_upsert` on the
`distill.rs` pattern and lets the graph extract. Push only `respond`/`notify`
buckets.

**The nightly's outage on 2026-08-19 cost three fixes**, all shipped, and they
are worth reading together because one absence produced all three:

1. The model server was not a unit (above), so it did not come back.
2. `mecha mail classify` returned `Ok(())` however badly it went, so a run that
   classified 0 of 16 logged SUCCESS and every exit-code-based check —
   `OnFailure=`, `systemctl --failed`, doctor's failed-unit scan — read a dead
   nightly as a healthy one. It now exits non-zero when a run classified
   nothing, disposed of nothing, and failed at least once. Partial failure is
   still success on purpose: failing the unit for 14-of-16 trains someone to
   ignore the alarm.
3. **The sweep skipped anything the store had heard of**, and the store holds
   failures as well as verdicts, so all 17 threads that failed against the dead
   server would have been skipped forever — including a manuscript review
   invitation, the category with the lowest reply rate and the hardest
   deadlines. `TriageStore::needs_classifying` replaces `!is_known`: absent or
   `failed` means classify, `dismissed` is a person's decision and
   `classified` is done. Nothing would have reported this one either — the
   sweep printed "0 to classify", which is what a quiet morning looks like.

**Local state in no repository**, and the next session will want it:

- The classify timer is installed at `~/.config/systemd/user/`.
- `~/.mecha/mail-triage/` holds 51 records classified across **four** binary
  generations now, so `request_type` is not consistent across the store and the
  taxonomy changed again on 2026-08-19. It wants one `mecha mail classify
  --account dartmouth --limit 50 --force` sweep (~25 min) before any
  measurement is taken from it. Five of those records carried
  `proposed: frontdoor` and now read as `none` — by design, not by damage.
- **The corpus is at `~/.mecha/mail-corpus/{dartmouth,personal}.jsonl`**,
  owner-only, outside the repo, `*.jsonl` gitignored so it cannot be staged.
  both accounts complete as of 2026-08-19. Do not re-fetch to re-run an
  analysis; do re-fetch if the window needs extending. The personal half was
  fetched twice — the first was truncated by a 500-per-month cap, now fixed and
  reported when reached.
- **The personal account should stay out of the nightly**, and this is now
  measured rather than assumed: it is far more machine-generated than the work
  account and carries almost no correspondence needing an answer. Adding it
  would roughly double the thread count for a negligible number of real
  threads.

### TUI polish

- **Steering and queuing are the same key.** Enter starts a run when idle and
  steers one already going; there is no way to queue a follow-up instead.
- **No `/export` or copy.** `NAMES` lists nineteen commands (2026-08-20, after
  `/docs`) and none of them get the transcript out. `/docs` does put a link on
  the system clipboard over OSC 52 (`tui/docs.rs`, `clipboard_escape`), which
  is the mechanism an export would use — it survives SSH because the escape
  travels back down the same connection the screen does.
- **`NO_COLOR` is honoured only by the plain CLI renderer.** The TUI hardcodes
  colours inline; there is no semantic colour table.
- **No keymap configuration.**
- **The "is a modal open?" guard is a list maintained by memory.**
  `open_scoped_review` enumerates every modal field to decide whether something
  already owns the keyboard, so it is wrong the next time one is added — and it
  was wrong three times over, missing `/mail` and `/polls` from the day they
  shipped until 2026-08-20, which meant a run finishing under `/review now`
  opened `/outbox` *underneath* the visible modal and drove it with keys nobody
  could see. Fixed by adding the three names, which is the fix that will be
  needed again. The shape that ends it is one predicate the modals answer
  rather than a list of fields.
- **`pub const NAMES: [&str; N]` states its length twice.** The count is in the
  type, so adding a command means editing two places — and when two sessions
  add one each, the merge is textually *clean* and the array ends up one short,
  because both sides made the same `18 -> 19` edit on one line and git takes it
  once. It fails as a compile error rather than silently, which is the good
  news. `pub static NAMES: &[&str]` says it once; `iter()` and `contains()` are
  unchanged. Deliberately not done on 2026-08-20 with a second TUI arc in
  flight: changing that line would have traded a known one-character merge fix
  for an unknown conflict.
- **Six modals scroll their detail unclamped.** `/skills`, `/outbox`,
  `/frontdoor`, `/triggers`, `/polls` and `/doctor` `saturating_add` the offset
  with no upper bound, so you can scroll past the end into blank space. You can
  still reach everything, so this is "cannot tell where the end is" rather than
  hidden content — the reason it is listed and not fixed. `/tools`, `/tasks`
  and `/help` were given the measured form on 2026-08-20 (`line_count`, clamp
  to `drawn - visible`, hint only when `max_scroll > 0`) and are what to copy.
- **Captured log lines wait for the idle tick.** `logs.rs` drains at the top of
  the event loop, which fires every 200ms during a run but only every 60s at
  idle, so a warning with nothing else happening can appear a minute late. Any
  keypress drains it at once. Ending it means a channel the writer wakes the
  loop on; at idle there is nothing running to warn about, so it was judged not
  worth the complexity rather than overlooked.

### The graph, now a sibling

mecha-graph shipped 2026-08-16 (repo public, three crates at 0.1.0, tools
unprefixed, store at `~/.mecha-graph/`). What that arc left open:

- **No release workflow.** The three crates were hand-published; the repo has
  no CI at all. mecha's tag-driven workflow with Trusted Publishing is the
  template, and the half-published-workspace trap it documents applies
  verbatim to a three-crate workspace with an internal dependency.
- **The dependency inversion is scoped and unstarted.** `vet`, `gossip`, and
  `corroborate` are graph-curation agents squatting in mecha-cli — the
  2026-08-16 gossip work needed paired commits across the two repos, which is
  the friction the move ends. The sorting rule: a command belongs to the repo
  whose store it curates; `distill` stays (it reads mecha's sessions and is
  the bridge). Move `gossip` first; it builds on published `mecha-core`.
- **A stranger-facing README pass.** The public README still reads like the
  private repo's; nothing in it walks a person from `cargo install
  mecha-graph` to a populated graph.
- **Cosmetic**: the private checkout still lives at
  `~/Github/personalized_knowledge_graph` (paths baked into mecha's config
  `command =`, two crontab lines, and the gitignored OPERATIONS.md), and
  mecha's CLAUDE.md still says "pkg" in narrative spots.

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
  config says why at length. Half the premise aged out 2026-08-16: the
  Landlock backend now confines without privilege on exactly this box, with a
  per-server `network` override for the vendor-Pyodide fetch — the decision
  deserves a revisit, and the config comment predates the option.

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

  **The personal public surface is merged and deployed** (verified against
  `main` 2026-08-16: switchboard `6fbd2c0`, cockpit records `ff91bfa`, the
  ten review findings `85be8b4` all on `main`; the box serves 0.2.4, which
  contains them). It adds two pages a stranger sees: the **hangar** at `gate…/@<handle>`, which lists everything that
  person has made public, and a **switchboard** at `gate…/@<handle>/<slug>`,
  a hand-patched page of lines meant for an email signature. The design is
  [`SWITCHBOARD-DESIGN.md`](SWITCHBOARD-DESIGN.md) (open here as
  [ljchang/mecha#41](https://github.com/ljchang/mecha/pull/41)); it is the
  authority and nothing about the arc should be re-derived from this
  paragraph.

  Three things a reader needs before touching it:

  - **It closed a live bug on the way.** Withholding is stored per *version*
    and visibility lives on the *alias*, so a public bundle whose live version
    an operator had withheld read "everyone" on its owner's account page and
    served nobody. `bundles_overview` reads the withheld column from the
    aliased version's own row now — the old query's `MAX(b.version)` answered
    about a version nobody serves.
  - **`inventory.rs` is the load-bearing piece, not the pages.** One query
    answers "what has this user got" across all four kinds (bundles, forms,
    booking pages, polls) and computes `Reach` — can a stranger open this —
    once, by the same reasoning `http/artifacts.rs` uses before serving a
    byte. The account page, the hangar and a switchboard all render from it,
    differing in a filter and never in a source. A second opinion about who
    may read what is how a private artifact ends up named on a public page,
    where the title alone is the leak.
  - **Schema 13** adds one `records` table holding a profile and every board,
    each as two texts: `baseline` (the TOML exactly as last pushed) and
    `effective` (what the page renders). The difference between them *is* the
    set of edits made in the cockpit, which is what a later push folds around
    rather than flattens.

  **A high-effort review ran over the branch and its ten findings are fixed
  (`f7350bb`), which took the suite to 467.** Three were reachable by a
  stranger or an injected instruction — an MCP slug that became a filename
  with no validation and skipped the `confined()` jail, `host_of` returning a
  URL's userinfo as its host, and a labelled profile link rendering with no
  host at all on the gate origin. Two were merge bugs: deletions never
  applied to a record first written in the cockpit, and the pull-then-push
  workflow announced data loss that had not happened. The two lessons worth
  carrying are in [`HISTORY.md`](HISTORY.md) — a doc comment is not an
  enforcement, and a static path segment shadows the parameter beside it.

  One decision the fixes made, which the design doc now records: **`theme`
  and `accent` are refused** rather than accepted and ignored. They validated
  and stored and no renderer read them. They return with the renderer, and
  §6.3 names the choice that has to be made first — an inline token block
  keeps the stylesheet handle-free where a per-user sheet would not.

  Left undone deliberately, each named in the PR:

  - **Board deletion does not exist.** A retired slug must never be reissued —
    the argument that makes a handle unreusable — which needs a retired-slugs
    table like `handles`. A delete that silently frees a name somebody has in
    an email signature is worse than no delete, so there is none.
  - **`--unlisted` is unbuilt**, so every public artifact appears on the
    hangar. That *is* the design's default (§3.3); what is missing is the
    per-artifact opt-out, which is a property on the bundle row rather than
    anything about the page.
  - **Avatar and per-line click counts** are §12.2 and §12.3 of the design,
    both past the core on purpose — the first pulls in the blob/upload path,
    the second is a new data class (visitor behaviour) on a box we assume is
    lost and needs its own retention answer.

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

    **The box runs v0.2.6 (deployed and verified live 2026-08-17)** — three releases past the v0.2.1 that first exercised
    `factory-deploy` end to end (download, checksum, prove, swap,
    health-check). The served stylesheet went from 23,397 to 30,939 bytes and now
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
    (claiming a handle, pairing, the five scoped keys — of which pairing
    installs two, corrected 2026-08-10 — and wiring the MCP surface into an
    agent), `artifacts.md` (versions, the alias, visibility, the
    share/revoke path and its oracle rules, takedown, what retention will not
    sweep) and `notebooks.md`. Also `features/slack.md` on the mecha side,
    which is setup plus what the remote control actually does.
    [`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md) lists the rest with the
    claims each has to make, sourced from the code. `overview.md` landed
    2026-08-10 with the documentation overhaul — both directions of the
    boundary, the three crates and the three origins, the request lifecycle
    end to end, and the notebook confinement gap stated as a warning. Still
    missing, in the order they bite: `field-kinds.md` — the four-column table
    (TOML · JSON Schema · rendered control · what the server enforces) exists
    nowhere but `request.rs`, and `second-client.md` assumes it; `booking.md`,
    since the whole scheduling instrument is still undocumented for readers;
    `request-types.md` and `theming.md`.

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
  - **It is installed and running** (`mecha-slack.service` active, re-verified
    2026-08-20 — but on the **0.1.8** binary, since the install is a release
    behind the repository).
    `mecha-slack.service` is enabled with linger, and `[slack] tools` is now
    `[]` — the whole surface, **including `mail__*` and the graph's `kg_*`**
    (re-measured 2026-08-10; the rationale comment sits above the line in
    `~/.mecha/config.toml`). The earlier stance that mail and the graph were
    "deliberately out" is reversed and this bullet is the record of that. The connector answers a
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
  `bench/run-subset.sh` runs the calibrated subset.

  **A complete scorecard exists as of 2026-08-11**:
  `jobs/mecha-arm64-subset/mecha-arm64-subset-2026-08-11__02-11-55/` (in the
  `bench-run-v012` worktree checkout) finished all 75 trials — **mean reward
  0.4533 (34/75), 8 errored trials, k=1, v0.1.2 binary, 262k window,
  `max_turns` 80, timeout multiplier 2.0**, `scorecard.html` beside
  `result.json`. The four falsifiable checks it was launched to answer came
  back: empty-turn nudges fell to 4 across the whole run (from a rate that
  poisoned the 08-07 fragment), and the dash-prompt crash did not recur.
  Treat it as the k=1 baseline for the 0.1.2 harness; the 08-10 28-trial
  fragment and the 08-07 attempt stay in their worktrees as diagnosis
  evidence, not baselines. Two earlier lessons still stand when launching:
  rebuild the portable binary first (`bench/run.sh` scores whichever
  checkout you launch from — last recorded at 0.1.6 on 2026-08-16 and
  **not re-verified since**, so assume it is stale rather than current), and read any
  job with `bench/check-subset.py` before believing it is a subset.
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
