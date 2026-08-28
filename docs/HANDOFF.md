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

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.15**
(2026-08-26 — five surfaces that described themselves wrongly, found by
using them: a chip claiming a stop-and-ask that would not happen, a card
printing raw JSON at somebody about to approve it, a graph verdict that
could fail with nowhere to go, a call that ended without saying why, and a
harness-change gate that read the label the proposer had written on its own
change; 0.1.14 shipped 2026-08-25 — voice calls and chat became one
conversation, three review surfaces stopped hiding what they were asking
people to approve, and the nightly mail classifier took both mailboxes;
0.1.13 shipped 2026-08-24 night with the web surface, voice, and the graph
queue's similarity groups; 0.1.12 on 2026-08-22, 0.1.11 and 0.1.10 both on
2026-08-21, 0.1.9 on 2026-08-20, and 0.1.7/0.1.8 on 2026-08-19/20 after the
mail hold lifted).
**`main` carries commits beyond v0.1.15 that are not yet tagged** — rung 9's
episode tagging, surprise detection and the appraisal-arc PRs before it, all
in the `Unreleased` section of `CHANGELOG.md` as of 2026-08-28.

**0.1.14 is thirty-one commits from three sessions working the same day**,
which is the thing to know about reading its history: the lanes interleave,
so `--ancestry-path` answers "what is in this release" and a date range does
not (see the Measuring trap that count produced).

**0.1.13 is a patch bump carrying two whole subsystems**, which is worth
knowing rather than discovering: 91 commits, including everything the voice
stack and `mecha serve` are made of. That follows this project's own
practice for 0.x — every release since 0.1.1 has shipped features under a
patch bump — but if the version line is ever meant to signal size, this is
the release that argues for it. **Four** crates are on crates.io — `mecha-core`, `mecha-mail`,
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

Expect **1,733 tests**, no failures — measured 2026-08-28 on `main` at
**be9e32b**, the merge commit of PR #101, the last of three rung-9 PRs (#97,
#98, #101) to land after the appraisal arc closed. Breakdown: **565** in
`mecha-cli` with 1 ignored, **943** in `mecha-core`, 6 + 9 in its two
integration suites, **133** in `mecha-mail` plus 1 in a mail binary, **75** in
`mecha-slack`, and 1 doctest. The **36** added over the previous figure
(1,697 at `a0638c8`) split `mecha-cli` +7 and `mecha-core` +29, with every
other suite unchanged — consistent with rung 9's episode tagging and
surprise detection (`appraisal::for_session`, `Distiller`'s new `Surprise`
extraction) plus #101's four rounds of terminal-escaping and outbox-read
hardening, none of which touch mail, Slack or the integration fixtures.

The previous figure was **1,697**, measured 2026-08-27 at **a0638c8**, the
merge commit of PR #92, the last of the nine appraisal-arc PRs (#86, #87,
#93, #88, #89, #90, #91, #94, #92) to land that day. The **131** added over
the figure two merges back (1,566 at `85b244c`, before any of the nine
landed) split `mecha-core` +99 and `mecha-cli` +32, with every other suite
unchanged — consistent with an arc that was almost entirely appraisal
logic, learning-UI panes and the replay surface store, none of which touch
mail, Slack or the integration fixtures. An intermediate count of 1,690 at
`c630ff9` (after #86, #87, #93, #88, #89, #90 but before #91, #94, #92) was
measured and superseded within the same session; it is not carried forward
as a separate step in this chain because nothing was ever built or read
against it.

Clippy and rustfmt are clean at that commit, **measured locally**. CI is a
separate claim and a weaker one, and it must be made per **sha**. Every PR
in the context-pressure arc (#66, #67, #69, #70, #71, #72) was checked with
`gh api repos/ljchang/mecha/commits/<sha>/check-runs` against its own head
commit before merging, all six jobs green — `gh pr view`'s rollup follows
whatever head the PR currently has, which is correct and moves, where a
sha-addressed query pins the evidence to a commit that cannot slide. The
merge commits themselves and `be75b73` are *unverified*: CI runs on pushes to
`main`, but nobody has confirmed the result on this sha.

That distinction is not pedantry; it cost an hour and a wrong sentence in
this file. Read Traps → Environment before writing "CI passed" anywhere:
three separate mechanisms were producing green-looking evidence for
untested commits on 2026-08-26.

**Measure the merge, not either side of it, and attribute with a commit
range** — the lesson of this particular count, and the reason those numbers
are `git diff v0.1.13..HEAD` per suite rather than a `git log` sweep. Three
sessions landed work the same afternoon; two wrote down a figure measured on
their own tip (1,365 and 1,367), neither describing the tree anyone would
check out, and the attribution sweep then ran inside a worktree cut 31
minutes before `5b187c5` existed, so three `mecha-core` tests had no lane.
Two wrong mechanisms were proposed for that before one was measured — they
are recorded in HISTORY under Traps → Measuring, along with the genuine
`--since` hazard that turned up while chasing it. Note also that the earlier
line said "2 doctests" where there is one plus a single test in the
`mecha-mail` binary; the total was right and the breakdown was not.

The previous figure was **1,343**, re-measured at the v0.1.13 tag
(068a659); the same number was measured earlier at f6a39a5, the
night of 2026-08-24, after the voice-controls and web-surface-arcs merges. The 20 over the count before it are
`mecha-cli` +13 and `mecha-mail` +7 — the phone surface's Arcs A/B/C and the
dictation route, plus the `mecha-mail` tests that the 08-24-evening count had
set aside as "an uncommitted docs WIP that is not this arc's to count" and
which have since landed. The previous count was 1,323, measured the evening
of 2026-08-24 in a clean worktree after the web-review-surfaces arc (680 /
429+1 / 122 / 75 / 15 / 2), and 1,281 before that, measured the evening of
2026-08-23
after the groups/cascade, clean-evidence-learner and graph-surfaces arcs (679
in the `mecha-core` lib suite, 388 in `mecha-cli` with 1 ignored, 122 in
`mecha-mail`, 75 in `mecha-slack`, 15 across the two integration suites that
need real backends, and 1 doctest). The 12 added over the morning's 1,269:
the clean-evidence reflector (4 in core), the cascade/Fan refusals and tally
(3), the groups envelope/parser and /find modal (3), and the Slack `note`
and `queues` command words (2). The morning's 14 over 2026-08-22's 1,255
were `mecha review`'s tally and refusals, the `/queues` modal's parsing,
tier filter, item detail and tiny-size draws, and `exe::self_exe`. The 9 added over 2026-08-21's 1,246 are the harness store,
override layer, and doctor's harness/starved-learner checks; the 24 before
that were the search-chain, cache-lens and outbox-approval arcs. Earlier 2026-08-21 counts were 1,222,
1,213, 1,210 and 1,192; the 2026-08-20 counts were 1,140 at `cfa2cc2` and 1,105 at 0.1.9,
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
| `mecha-core` unit | 943 |
| `mecha-cli` unit | 565 (1 ignored) |
| `mecha-mail` unit | 133 (+1 in the `mecha-mail` binary) |
| `mecha-slack` unit | 75 |
| integration (`mcp_server` 6 + `sandbox_backends` 9) | 15 |
| doctest | 1 |

Measured 2026-08-28 at `be9e32b`, same tree as the prose above. The table
had drifted two counts behind that prose once already, which is the failure
mode of stating one fact twice — read the prose if they ever disagree again,
and fix the table.

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
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — a preference-ordered chain, first to *answer* wins. An exhausted chain of genuine empties is an answer; only a chain where nothing answered is an error. `prefer_deep` promotes a backend to the head on `depth: "deep"`, reordering and never filtering, so every backend stays reachable as a fallback at either depth |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Sandbox | `shell` and MCP servers confined via bubblewrap, docker, or landlock (no-privilege file confinement; never narrows `external_send` — UDP is unrestrictable, and the preflight plants a home file and requires the confined read to fail) |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Context | Two-pass compaction: thin tool results, then summarise. Taint preserved, a tool's own state (the todo list) crosses verbatim, the states a mid-run rewrite replaced ride `Conversation::rewritten` into the session record, and a per-run cache lens watches whether the cached prefix is actually reused (warns only on unexplained re-payment) |
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `review` / `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep, `slack` for the remote control, and `serve` for the tailnet web surface |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning. Review lives here too: `/queues`, `/outbox`, `/frontdoor`, `/mail`, `/tasks`, `/skills`, `/polls`, `/doctor` and (2026-08-23) `/find` modals drive the CLI like `/triggers` does — plus `/note` for one-line graph capture — the status line badges pending drafts, and `/review now\|later\|auto` decides what happens when a run stages some — scoped to that run's items by an id-diff, tainted drafts never auto-released, the mode set only by command (never parsed from the prompt). Detached releases/extractions/triages are watched and their results reported without a reopen |
| Slack | `mecha slack` — a remote control: Socket Mode from home, an owner allowlist bound by a locally printed nonce, a thread as a `Conversation`, streamed answers with a task card per tool call, approval cards (incl. "allow for this run"), outbox review cards, files both ways, `notify`; owner-gated command words `doctor`, `triggers`, `review now|later|auto`, and (2026-08-23) `note <text>` — a deterministic capture matched before the text can become a prompt — `queues`, the read-only backlog rollup, and (2026-08-24, adopted from an orphaned WIP) `tasks`/`task`, the GTD board as command words. **Merged 2026-08-09 (PR #25) and running as `mecha-slack.service`** |
| Web surface | `mecha serve` (2026-08-24, extended the same evening) — the tailnet web app: binds 127.0.0.1 with no flag to widen it, `tailscale serve` is the door (`:8443`), every request must carry `Tailscale-User-Login` equal to `[web] owner_login` (global-file-only config, stripped from project layers like `[slack]`; refuses to start ownerless), strict self-only CSP. Pages: Home dashboard (`review queues --json` + `doctor --json` as child processes, a dash never a zero), streaming chat over SSE (one shared agent, per-session `RunContext` on the Slack connector's pattern — keyed sessions with validated directory-safe names, jails under `~/.mecha/work/web/<key>/`, steering, cancel, context gauge), outbox review (whole `DraftView`, source reads behind a gutter, taint sheet with exact args, approve `--yes`/reject/edit as CLI children), graph-queue sample deck (seed printed, verdict ≠ resample), tasks, notes + `kg search`. Per-session `ask` mode: live approval cards (deny-with-reason is a real user correction; timeout is `Blocked`) and `ask_user` option cards routed by the run's jail (`Asker::ask_in`); pending cards ride the transcript read so a locked phone reloads into them; cancel drains parked cards. Evening additions (2026-08-24): the **mail page** (`serve/mail.rs` + `Mail.svelte` — store read for the list, `mecha mail show` as the one thread renderer behind a gutter, closed-verb `/api/mail/act` with spam the only confirm; drafting verbs spawn detached into the outbox), the **graph queue at all three depths** (classes with server-stamped tiers from `tui::queues::Tier::of`, per-class similarity groups, and the cross-class global layer with a threshold stepper — see the graph repo's `similar.rs` for the invited-crossing rules), and **files** (`serve/files.rs`: uploads into the session jail's `inbox/` announced as paths, downloads that re-prove containment, images the only inline type). Assets are a build artifact at `~/.mecha/web/dist` (update skill surface 1b). **2026-08-25**: `review now` reached this surface — a finished run emits the ids it staged and the page draws a confirm card built from `/api/outbox/{id}` (whole `DraftView`, taint above everything, source reads behind a gutter, Send now / Later); notes open in place and edit through `mecha kg note --edit <source_id>`, which preserves the note's own `occurred_at`; and a failed graph verdict keeps its card and offers the two ways through (`bind`, accept-as-new-topic) that until then existed only in the TUI. **2026-08-26**: the board learned to delegate — *ask mecha* (`/api/tasks/work`, detached and unattended, because `serve`'s approval cards belong to a chat session's `RunContext` and cannot reach a child process), *stop* (`/api/tasks/stop`), *open the conversation* (`#chat/<session>`, which `Chat` resumes from the route), an agent chip derived from `waiting_on` rather than self-reported, `task:` sessions in the drawer, and the plan rendered in chat (live, from the shared `TodoTool` keyed by jail) and on the card (`/api/tasks/plan`, read out of the transcript because a `tasks work` run is another process). **2026-08-26 (second pass)**: the delegation loop closed — `/api/questions` (list, answer, abandon) so a parked question is answerable from the phone rather than only from a terminal, and the task card's state derived from board + questions + the transcript's outcome record (D16). **2026-08-26 (fourth pass)**: the graph queue became reviewable rather than only tappable — a group opens to its members (`GET /api/queue/items`, a named id set re-fetched by id, never a redraw) and each is verdicted on its own with no cascade, because similarity is the grouping key and members can contradict each other; a candidate's face comes from one `faceOf()` matched to `tui::queues::items_from_json`'s chain (`statement` → `what`), which is what stopped every commitment card rendering `undefined — undefined — undefined`; and a failed *bind* offers a target field, distinct from a failed accept, where naming a target is not the answer. `docs/REMOTE-SURFACE-RESEARCH.md` + `-DESIGN.md`, `docs/TASK-AGENT-DESIGN.md` |
| Voice | The stack from `docs/VOICE-RESEARCH.md`, built and in production 2026-08-24 (§7 is the build log): Pipecat worker (`scripts/voice/worker.py`, `:7860`), **Parakeet TDT** STT (`mecha-parakeet.service`, `:8992` — Voxtral was structurally unfit: a chat model answers speech instead of transcribing it, and obeys spoken instructions), Chatterbox TTS (no standby — Kokoro was removed 2026-08-25; nothing failed over to it automatically), and the loopback OpenAI facade (`mecha-cli/src/voice/`) **mounted inside `mecha serve`** (`--voice-port 8990`) over the shared agent — one process, one cached prefix, two dialects. The WebRTC offer proxies same-origin through serve (`/api/offer`), behind the owner guard — true of **both** doors since 2026-08-25, and only of `:8443` before it, when `:443` was a file mount whose `/api` went straight to the worker. In-chat voice: waveform button → call overlay (voice-core.js embedded by relative import; threaded transcript pane, cloned-track mic meter, mute, end). **A call is the chat session it was started from (D3, 2026-08-25)**: the page names its key in the WebRTC offer (`request_data`, pipecat's own passthrough), the worker forwards it as `X-Chat-Session` beside the slot key it still mints, and `voice::SessionHost` — implemented by `serve::chat::VoiceHost` — runs the turn on that conversation's messages, taint, transcript and jail, with the facade keeping no record of its own. `chat::begin_turn` is the one implementation both doors go through. Spoken turns arrive on the page's SSE feed live (`WireEvent::User`, block stripped) and are marked `spoken` in the transcript; the D10 block now opens a *switch into speech* rather than a conversation (`last_turn_spoken`), and `--voice-yes` travels with the turn, so a spoken turn runs at Allow while a typed one in the same session obeys the page's mode. D5 ratified: owner speech is typed text, arms nothing. **Voice controls (2026-08-24 night):** the in-chat call overlay (`Chat.svelte`) carries a **seven**-voice picker and a 0.5–2.0x rate slider. They persist in `localStorage` (`mecha.voice.prefs`, `{voice, speed}`, read on each connection's first `onVoiceConfig`), so the next call opens where you left it rather than resetting to whatever the worker booted with. That key originally synced two shells; the standalone page was retired in 876580e and the preference is why it stays. Seven is six Kokoro-derived cloning references plus Chatterbox's own built-in `default`, which the server lists as selectable because it is one — `voice: "default"` generates with no reference rather than falling back to anything. The controls are driven by a `voice-config` RTVI message and `session.voiceConfig(patch)` on `voice-core.js`; the server's reply is what renders, so a refused value never leaves the control showing a rate the worker is not speaking at. Rate is a pitch-preserving phase vocoder in `chatterbox_server.py` (~50 ms warm) because Chatterbox Turbo has no speed parameter and resampling moves pitch with tempo. The voices are Kokoro presets synthesized into cloning references by `scripts/voice/make-voices.py` — Apache 2.0, nobody's identity — and the server reads the directory live (`GET /v1/voices`) rather than holding a list. **Call teardown, 2026-08-25:** the pipeline's idle timeout was pipecat's unchosen 300s default and killed a call five minutes into any pause — raised past a conversational silence and it now *announces itself* over the data channel before tearing down; client-side, ICE `disconnected` is a 15s grace window rather than a hang-up, since only `failed`/`closed` are terminal (no ICE restart: pipecat's `restart_pc` fires the very event this worker cancels the pipeline on). **Spoken outbox confirmation, 2026-08-25:** a run that stages drafts is asked about aloud — the offer composed from the store through `DraftView::spoken`, the answer matched by `review_policy::parse_answer` *before* any model sees it, so the release decision never enters a context window |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit (`--body-file` for surfaces with no `$EDITOR`)/**review**/send/reject, several ids or `--all` narrowed by `--kind`/`--via`; edits mined as writing reflections. Items carry a kind — a publish shows its rendered page, refuses `edit`, and is excluded from the miner — and the jail they were drafted under, so a release resolves paths against the agent's workspace rather than the reviewer's. Release policy is `review_policy.rs` — one encoding for every surface: `now` (the default) puts a finished run's drafts in front of you, `later` leaves them, `auto` releases only untainted drafts of a run that finished clean. Scope is `staged_since`, an id-diff, so no mode touches items another run staged. Since 2026-08-25 `now` reaches `mecha serve` too — a card on the page, a spoken offer in a call |
| Messaging | `[messages]` + `mecha msg send/list/show/dismiss/agents` — a file mailbox between this machine's sessions (`~/.mecha/messages/<recipient>/`, producer-name addressing, per-session liveness registry). Delivery folds in at the steering point with the sender's harness-stamped taint merged first, so a hop launders nothing; attended surfaces hold with a notice, unattended accept; global config only; full mailboxes refuse, identical pending sends dedup. `docs/MESSAGING-RESEARCH.md` is the design record; phase 2 (TUI modal/badge) is scoped there |
| Workspaces | `~/.mecha/work/<producer>/` is a run's workspace and its output directory; `mecha work list/path/clean`, retention nightly. A workspace containing the mecha home is refused |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed. **`mail_triage`** (2026-08-18) adds archive/read/unread/spam/trash as a closed `TriageAction` enum, thread-level, in a third capability quadrant — `destructive` but *not* `external_send`, so it never routes through the outbox and a read-only run cannot reach it. Tagging is deliberately absent: a tag is mecha's own, on the triage record, not a Gmail label or a Graph category |
| Tasks | `mecha tasks` list/add/set and the `/tasks` modal onto the graph's GTD board, reached only over `kg_task_*` — no dependency on mecha-graph and no second reader of its schema. Status letters match `mecha-graph tui` screen 6; nothing confirms (the board reaches nobody and has no delete); a reload re-finds the cursor by id because a status change reorders the board. **2026-08-26**: `tasks work <id>` seeds an agent run from a board item (own session titled `task: …`, outbox-bound, `--unattended` for the trigger posture a detached caller gets) and `tasks stop <id>` asks it to stop through a sentinel it polls — keeping the partial turn, never a kill. The harness moves `waiting_on` to `mecha` for the life of the run and to `@owner` when it ends, so "is work happening" is answered by the board rather than by the run; `kg_task_update` is withheld from the model for the same reason `kg_accept` does not exist, and `setup::subagents_holding` refuses to start when a profile would hand it back. `tasks set --waiting-on` is the hand-driven half. **D16 (2026-08-26, second pass)**: the card's state is derived from three sources and none of them is the run's account of itself — the board says who holds the ball, the question store says whether it is blocked, and the transcript's `Record::Outcome` says how the last run stopped. Seven states, no two rendering alike: `working` (with the `[~]` item as its subtitle), `planning`, `answer needed`, `ready for review` (with its evidence — turns, calls, staged, refused), `the run failed`, `outcome unknown`, and quiet. `unknown` is the honest seventh rather than a hedge: a transcript with no outcome record is a run that never got as far as saying how it went, and calling it either `failed` or `ready` invents the one fact the card is about. `Interrupted` reads as ready, never failed — a person stopping a run is the system working. **Provenance** (2026-08-26, a second lane): a task carries `captured_from` — a *pointer*, kind from a closed set of `mail | frontdoor | session` with unknown keys refused, so it can never become a copy of an email body. `mail task` writes it at capture, `mecha tasks source <id>` follows it with one reader per kind, `POST /api/tasks/source` and the card's `read the …` control do the same from the web, and the TUI offers `o` from the detail pane only — off the key strip, because it is inert on any task somebody typed. **B1/B2 (2026-08-26 evening, both amended before building)**: `✓` is a tap on the collapsed row rather than a chip in the strip (Things: complete is "a tap on the circle only"), and the expanded card *groups* under `hand it over` / `move it` instead of hiding four chips behind a `…` — the survey it cites is about swipe actions on a *collapsed* row, and this card only exists once tapped, so it already is the sheet. Capture is **one box**: `capture::find_when` parses a `when` out of the sentence in Rust, shows it as a dismissable chip, and sends the name **verbatim** (Things, not Todoist), with due/context behind `more`. It detects and never resolves — the span goes to the graph's `parse_due`, so one date parser lives where `+3d` means something; weekdays are deliberately undetected because `parse_due` cannot take one and a chip that lies is worse than none. There is no time of day anywhere in the store, so "at 3" stays in the name |
| Questions | `mecha_core::questions` + `mecha questions list/show/answer/abandon` — the outbox's inbound twin. A delegated run that needs a decision **ends**: `ParkingAsker` stores the question and cancels the run's own token, so the partial work survives and no slot is held waiting. Answering *is* resuming — the answer becomes the next user turn of the session that asked, in the jail it asked from, with its plan restored. Taint is recorded at park time and unknown reads as untrusted, because a question is an inbound request for information composed by a model that may have been reading third-party text. Sixth row in `/queues`; doctor flags one unanswered past 24h. **2026-08-26 (second pass)**: the phone can answer one — `GET /api/questions` is a direct store read (mecha's own store, so `review.rs`'s pattern rather than `board.rs`'s CLI child), the card lands on its task in `/tasks` because D13's own argument is that the Waiting view *becomes* the queue of blocked delegations with no new noun, options are one tap and the free-text box is there because the tool's contract says they are never exhaustive, the taint marker travels (stderr's `⚠` is invisible in a browser), a question whose task is off the board still gets a card, and answering spawns `questions answer --unattended` detached because answering *is* a whole agent run |
| Graph reads | `mecha kg search\|entity\|note` (2026-08-23) — the graph for the person at the keyboard, over the same `kg_search`/`kg_entity`/`kg_upsert` surface the model uses. `/find` is the modal (entities open their full record, facts/episodes open in place, `/` re-edits the query); `/note` (or `/notes`) captures an episode with entities linked on landing, identically to `mecha-graph note`. All fetches off the event loop through watches |
| Documents | `mecha-docs`, the fourth binary on `mecha-mail` — Google Docs/Sheets/Slides under **`drive.file` and nothing else**, so only files mecha created or the user picked in Google's own chooser are reachable, and no instruction inside a run can widen that. Reads are `untrusted_input` and never `openWorldHint`; writes are outbox-routed, because writing into a document a third party can read is a publish. No permanent-delete and no sharing verb, with tests on the absences |
| Remote control | `/remote-control <name>` in the TUI mirrors a live session into a named Slack thread, both directions. Store `~/.mecha/remote/<name>/` (record + inbox + staged files), written by the TUI and read by the connector, which no longer starts its own run in a mirrored thread. Out: `/send <path>`, `mecha slack send`, and the `show_file` tool — whose destination is not an argument and cannot be made one. In: attachments land at `./inbox/`, announced as paths so the taint arms through `fs_read`. Slash commands and `!` stay at the terminal. `mecha slack remote [--sweep]`. `docs/REMOTE-CONTROL-DESIGN.md` |
| Front door | `mecha frontdoor` list/show/extract/next/**triage**/**needs-info**/**close** over `~/.mecha/requests/` — the quarantine between a stranger's request and a run with tools, and the state machine that lets one reach an answer. The extractor is issued no tools and no history; `Record::for_privileged_run` has no argument that returns the prose; an extraction failure routes to a human. `triage` drafts into the outbox and refuses to run unrouted; `reconcile` closes the loop from released items on its own, with no verb to remember. `mecha-factory-publish drain` fills the directory |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, **the daemon is installed and running here**; a failed `notify` is recorded on the run |
| Skills | `~/.mecha/skills/<name>/SKILL.md` in the Agent Skills format, loaded by a `skill` tool call at three levels of disclosure. User-authored with no mechanism for anything else — no install, no registry, no remote body, none derived from a session — which is why loading one arms no taint. `tools:` narrows the surface and can never widen it; a loaded skill crosses compaction verbatim; `mecha eval` forces them off |
| Learning | the full arc: reflect-on-close → nightly rumination → counterfactual validation (steers/denials trace-graded) → gated proposals (`mecha proposals`); git-backed store under `~/.mecha/learning`; rules carry id/sources/created_at, validate feeds a per-rule outcome ledger with regression bisection, and `mecha rules` retires through the same gate (`eval --ab-rules` for the coarse A/B). Budget is 25 active rules and 2600 chars **per domain**, and a run carries only `RUN_DOMAINS` (`behavior` + `writing`) — new domains are opt-in and `unrouted_domains` warns at startup on any that ride in no prompt |
| Run quality | `Record::Outcome(RunStats)` per finished run from every front-end; `runlog.rs` reads the corpus back (`mecha sessions health`, rates split by model, `—` where a denominator is zero); three population checks in `doctor`; `candidate.rs` gates a proposed change on a paired comparison with a deterministic holdout and a work guardrail; `mecha eval --ab-config KEY=VALUE` is the content-sensitive arm; `mecha diagnose` proposes one change from the corpus and prints the command that would falsify it; `mecha harness` (2026-08-22) closes the loop nightly — candidates persisted, measured by session replay, a holdout-confirmed config win auto-accepted into a revertible override layer beneath the user's config, everything else staged for review — see the self-improvement section |
| Queues | `mecha review` + the `/queues` modal — every store waiting on a human in one list: the graph's merge queue, the outbox, the front door, staged rule changes, harness candidates. Four hand off to the modal that owns them; the graph queue is reviewed in place, four levels deep (mechanism → class → similarity groups or a *random* sample → items), `t` filtering by evidence tier and `a`/`r` verdicting. `s` on a class groups its whole queue by semantic similarity (2026-08-23) — every pending item, largest groups first, singletons after — and a group verdict is ONE human verdict: the seed is the owner's, members cascade machine-labeled (`reviewed_by = cascade:<seed>`, invisible to the ladder) onto the exact ids that were on screen; `b` binds a group's subject, `A` accepts creating it, `[`/`]` re-group at a threshold stepped from the value the child reports. An unreadable store prints a dash, never a zero. **2026-08-26**: `B` names a bind target by hand and a failed `b` opens the same prompt — the graph answers an unbindable subject with *name a target with `--to`*, and until then no surface could send one; the prompt owns the keyboard while it is up, because `a`/`r`/`d` below it are verdict keys. Same day, `items --ids` stopped being trimmed to ten by `--top`, which had capped the group dive since the level shipped. **The one place mecha shells out to `mecha-graph`** — the MCP surface has no `kg_accept` and must not gain one |
| Eval | 36 cases, 15 tags (both re-counted 2026-08-20, unchanged), scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `graph-cases.jsonl` — 10 memory/interlock cases against fixture MCP servers (`--mcp-file`), renamed with the graph and expecting the bare `kg_*` names production serves (scorecards across the rename are not comparable) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

> **This checkout is a live service's `ExecStart`. Do not `git stash`, `git
> checkout --`, `git restore` — or `git checkout <branch>`.**
> `~/.config/systemd/user/llama-local.service` names
> `/home/ljchang/Github/mecha/scripts/start-moe-mtp.sh` **literally**, so the
> file in this working copy *is* the server's launch command. Anything that
> rewrites it — a branch switch, a revert, a stash — changes how the model is
> served on the next restart, and it comes back healthy and merely behaves
> differently, which is the worst way for it to be wrong.
>
> As of 2026-08-21 the clone is on **`main`** and the script is **committed**,
> so the sharper 2026-08-20 hazard (a parameterised but uncommitted script that
> a routine `git checkout main` would have reverted) is retired. The standing
> rule is not: **work on `main` from a separate worktree** and leave this
> checkout where the unit points. That is the correct application of the
> worktree advice rather than a contradiction of it — the arc that owns the
> service-referenced path stays put and everyone else moves.
>
> The general lesson is in HISTORY under Environment: **"move it to a worktree"
> assumes nothing outside the repository points at a path inside it**, and here
> systemd does.

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Re-verified
2026-08-21**: 8080 up with `total_slots=4`, `n_ctx` 262,144 per slot and
`modalities.vision: true`; 8081 up serving embeddings; 8082 and 8083 down;
SearXNG up on 8888. `~/.cargo/bin/mecha` was reinstalled 2026-08-21 (03:33, then again at
~15:00 and ~17:00 for the search and outbox arcs) and the three long-running
services restarted onto it each time.

**2026-08-23 evening**: `mecha`, `mecha-graph` and `mecha-graph-mcp` all
reinstalled from that day's HEADs (several times; the `update` skill's
stale-process sweep run after each) and the three long-running services
restarted onto the final install. The graph DB migrated to V017
(`reviewed_by`). Operationally pending: **one class has an earned ladder
promotion unapplied** (`mecha-graph ladder --promote` shows and applies it),
and the graph queue stood at **6,569** after the day's review work, down
from 7,296 at breakfast; by the evening of 2026-08-24 it read 7,505 in
`review queues` (the nightly proposes faster than class-at-a-time review
clears), which is what prompted the global similarity layer.

**A version string is not evidence here.** The repository and the binary both
say 0.1.9 and the reinstall changed no version, so `mecha --version` cannot
distinguish a current install from a skipped one — check a behaviour the change
introduced (`mecha run --help` carrying `--image`), and check
`/proc/<pid>/exe` for `(deleted)` to catch a *process* still running a replaced
inode. The `update` skill carries both checks.

**2026-08-24 (re-verified after the 0.1.13 release, ~21:45)**: `mecha
--version` reports **0.1.13** from `~/.cargo/bin`, `mecha-mail` reinstalled
beside it, web assets rebuilt and rsynced to `~/.mecha/web/dist` (the served
door returns the current bundle), and `mecha-serve`, `mecha-slack`,
`mecha-triggers` and `mecha-drain` all restarted onto the new inode and
logging their startup lines. **The graph binaries were deliberately NOT
reinstalled**: another session was mid-arc in the private graph checkout
during the release freeze, and installing `mecha-graph`/`mecha-graph-mcp`
from a working tree in flight is worse than leaving a known-good install
alone — so `~/.cargo/bin/mecha-graph*` was that session's to update, not
stale by accident. **That arc has since landed** (graph main `f312eb0`) and
both graph binaries are installed from it and verified answering twelve
`kg_*` tools. The carve-out paid for itself: checking install times against
the merge is what surfaced `mecha-graph-mcp` sitting an hour stale behind
`mecha-graph` — two crates, two installs, and the MCP one is the only one
mecha reaches at runtime. The update skill now says so in as many words. One `mecha tui` started ~15:10 is still on a deleted
inode with its three MCP children; it degrades gracefully since 0.1.12 and
needs its owner to restart it. Also not refreshed by this pass and
independently stale: the musl benchmark binary (`target-musl/release/mecha`,
built 2026-08-23), so **re-run `bench/build-portable.sh` before trusting any
scorecard against 0.1.13**. The factory client is current at 0.2.7 and the
droplet was not touched.

**2026-08-24 (re-verified this date, evening)**: `~/.cargo/bin/mecha`
reinstalled again for the web-review-surfaces arc (final at merge `cfab345`)
and `~/.cargo/bin/mecha-graph` + `mecha-graph-mcp` reinstalled from the
private checkout's `global-similarity` merge; all mecha-binary services
bounced onto the final inodes. One `mecha tui` (started that evening) was
seen on a `(deleted)` inode and left running — it degrades gracefully since
0.1.12 but is stale until its owner restarts it.
Production is now **one `mecha-serve.service`** (user unit, `mecha serve
--voice-port 8990 --voice-yes`) owning `:63242` (web, fronted by `tailscale
serve :8443`) and the loopback voice facade on `:8990`; `mecha-voice-serve`
is retired (disabled). New units beside it: `mecha-parakeet` (`:8992`, STT)
and `mecha-voice-worker` (`:7860`, Pipecat, reached through `mecha serve`'s
own `/api/offer` proxy rather than directly). Web assets live at `~/.mecha/web/dist`
— a build artifact, not in git; the `update` skill's surface 1b rebuilds
them. 8080 re-verified this date: `total_slots=4`, `n_ctx` 262,144/slot,
vision true. **Re-verified 2026-08-25** and all four unchanged
(`total_slots=4`, `n_ctx` 262,144/slot, `model_alias qwen3.6-35b-a3b`,
`modalities.vision` true). Workspace tests **2026-08-26: 1,408 pass, 0
fail** (`main` at 8ce53dc, clean tree; 1,377 at 897bd13 the previous day).
Eval: 36 cases, 15 tags, plus 10 graph cases — all three re-counted
2026-08-26, unchanged. The `[web]` section is live in `~/.mecha/config.toml` —
safe now that every installed binary parses it; the outage its early
arrival caused is in HISTORY under Traps → Environment. A `llama-voxtral`
unit still exists at the *system* level (voice arc's; healthy per their
2026-08-24 check, no longer the STT seat) — query it with plain
`systemctl`, not `--user`, or it misreads as inactive.

**2026-08-26 (00:12 UTC) — installed and restarted again, and one commit
of deliberate skew.** `mecha` reinstalled and all five long-running units
restarted during the day's work, each verified by its own startup line;
`mecha-graph-mcp` reinstalled from the private repo (12 tools, answered from
`~/.cargo/bin`) after `kg_notes` gained `source_id`; web assets rebuilt and
rsynced to `~/.mecha/web/dist`, confirmed by the hash the `:8443` door
serves. The stale-process sweep ran twice and found two the first time
(`mecha trigger daemon` and `mecha slack connect`, both cleared by restart)
and none the second.

**The running binary is one commit behind `main` on purpose.** The last
commit (`874eb6a`) boxes an error type to satisfy a clippy lint the CI
toolchain has and this box's does not; it is behaviourally identical, and a
second session was live in this checkout with uncommitted work in
`serve/chat.rs`, `serve/present.rs` and `Chat.svelte`. Reinstall and restart
`mecha-serve` next time the tree is not shared. **`docs/OPERATIONS.md` gained
the voice worker's real `--allowed-origins`**, which the tracked unit no
longer carries — anything reinstalling that unit from the repo copy has to
substitute them back.

**2026-08-26 — the live graph store was repaired in place, and no repository
records it.** `~/.mecha-graph/graph.db` is not in any git history, so this
paragraph is the only account of it. `mecha-graph repair-id-payloads` merged
**30** placeholder topic nodes into the nodes they were named after, rewrote
**121 of 8,988** pending candidate payloads from node ids to names, and
re-pointed **23 accepted facts** at real entities. The merges are the
irreversible part (`merge_nodes` has no clean undo) and were run on the
owner's explicit ruling, declining a backup first. A second `--dry-run`
reports 0/0/0, so the repair is idempotent and re-running it is the check.
What it does *not* undo: those 23 facts were accepted by a human at a time
when their subject stood for nothing, and they now say what their own
statement always said — a repair, and also a belief changing what it is
about.

**2026-08-25 (13:20) — everything installed and restarted at v0.1.14, and
the version skew that stood earlier today is closed.** `mecha --version`
reports **0.1.14** matching the tag; `mecha-mail` and its three sibling
binaries reinstalled at the same version; `mecha-graph` and
`mecha-graph-mcp` both reinstalled from the private repo at 0.1.3 —
**two crates, two installs**, and the MCP one is the only binary mecha
reaches at runtime (12 tools, answered from `~/.cargo/bin`). Five
long-running units restarted (`mecha-slack`, `mecha-triggers`,
`mecha-drain`, `mecha-serve`, `mecha-voice-worker`), each verified by its
own startup line rather than by `is-active`. Web assets rebuilt into
`~/.mecha/web/dist` and confirmed by the hash the `:8443` door actually
serves. The stale-process sweep found none. `factory-publish` and the
droplet are both current at 0.2.7 — checked, not assumed — and the
`mecha-sandbox` image's cargo matches the host's 1.97.1, so neither needed
touching.

Earlier the same day this paragraph recorded the opposite state, and the
reason is worth keeping: the installed binary was built from main while
the workspace version still said 0.1.13, so `mecha --version` reported a
number the bytes had outgrown — the "a version string is not evidence"
trap arriving through the door that looks most like evidence. Cutting
0.1.14 is what made the version line true again, not a fix to the check.

What has landed on main since the tag, beyond the `--json` flags and the
`/queues` review level: **`/entity` merges two nodes you pick yourself**
(`m` marks the survivor, `m` again confirms with `y` — the only key on that
modal that confirms, because it is the only irreversible one), **Esc peels
one layer at a time there** (merge → edit → search → modal), and **every
`/queues` row reports how long its oldest item has waited** beside its depth.
That last closed a real gap: four outbox drafts sat five days behind a row
reading `6`, visible only to `mecha doctor`. Depth is a snapshot; the surface
built because a queue reached 6,434 items unnoticed could not show a queue
growing.

**`mecha-slack` and `mecha-triggers` are on a deleted inode, and that is
intended.** Both read `(deleted)` on `/proc/<pid>/exe`; nothing landed today
reaches either, so the `update` skill's stale-process sweep will flag two
false positives. Restart them on the next change that actually reaches them,
not because the sweep says so. `mecha-serve` **was** in that set and is not
any more (pid 2626147, live inode): the outbox source join and the mail
resolver both run inside it, so it earned the bounce that the earlier changes
did not. The rule this pass confirmed twice: only `mecha-serve` shells out
through `/proc/self/exe`, and the `mail-classify` timer needs no restart at
all because it spawns a fresh `mecha` per run and picks up an install the
moment it lands.

**`:443` now serves the app, and the standalone voice page is gone.**
`tailscale serve` proxies both `:443` and `:8443` to `127.0.0.1:63242`, so the
two doors are the same surface; `scripts/voice/page/` was deleted and
`voice-core.js` moved up to `scripts/voice/`. Verified live this date from
`tailscale serve status`. The docs site was rebuilt but **not deployed**, so
the live site served "two places to talk from" until it was pushed. **Deployed
and verified this date** by fetching
<https://docs.mecha-factory.ai/docs/features/voice> rather than by reading the
workflow's green check — the phrase is gone and the app-only wording is
serving.

**The worktree convention has no retirement policy, and it cost 85 GB.**
Fourteen worktrees had accumulated under `.claude/worktrees/` and various
session scratchpads — essentially **one `target/` each** (17 GB in
`tasks-arc`, 11.7 in `vision-arc`) against roughly 100 MB of combined
source. The convention that produced them is right and is what let three
sessions work in parallel on 2026-08-25; what it lacks is any statement of
**when a worktree stops earning its disk**. Note the shape is already solved
one directory over: `[work] keep` answers exactly that question for
generated output, with `mecha work clean` as the policy and the nightly as
the trigger — which is why `~/.mecha/work` stayed at 56 MB while the
worktrees grew unwatched. `CARGO_TARGET_DIR` pointed at one shared directory
is the structural fix if it recurs; a retention verb is the policy one.

**And clearing them nearly destroyed the only copy of the raw benchmark
trials.** `results/` in main tracks the Terminal-Bench *scorecards*; nothing
tracked the per-trial output, and 1,627 files of `jobs/` — `config.json`,
`result.json`, per-trial `trial.log` from 2026-08-11 — lived only in the
`bench-run-*` worktrees, gitignored. Archived to
`~/.mecha/archive/bench-jobs-2026-08-11.tar.gz` (4.4 MB, file count verified
against disk) before removal. The near-miss is instructive because the rule
was already written down: HISTORY's "a gitignored file in a disposable
worktree dies with the worktree" entry predates this by weeks, and the
check that caught it was counting files on disk rather than trusting a
survey of tracked content.

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, **`total_slots=4`**, `-c 1048576` → **262,144 per slot**, and **`--mmproj` loaded since 2026-08-21 so `modalities.vision` is true** — the per-slot figure is the model's whole trained window (`qwen35moe.context_length`), raised from 32768 on 2026-08-10 after re-measuring. **`-c` costs nothing in speed**: 32k/64k/128k/256k are within noise of each other (~92 tok/s at a 1k prompt, ~80 at 30k), and the 50x slowdown recorded on 2026-08-07 was that day's OOM, not the flag. It costs memory as a startup *reservation* — 21.4 GB at 32k to 28.5 GB at 256k, i.e. weights ~20.7 GB plus ~32 KiB/token. **The full tables, the needle test at 188k, the `-np` trade-off and the two traps live in `scripts/start-moe-mtp.sh`** — read it before touching any of this. **`--reasoning-budget 4096`** (2026-08-07) was believed to be the mitigation for this model's "non-terminating reasoning" — **that diagnosis was wrong and is retired as of 2026-08-10 evening**: the empty turns were tool calls emitted before `</think>` closed, one of them 120 characters long, so no token budget was ever involved. The flag is harmless and stays; the real cause and fix are in `CHANGELOG.md` under 0.1.2. The nudge-retry allowance still resets on productive turns, which remains correct for its own reasons. `~/.mecha/config.toml` and `bench/mecha_agent.py` carry `context_window` and `max_tokens` (**above** the budget; 8192) — four numbers that move together. **`context_window` is `-c / -np`, not `-c`** — llama-server divides the context across slots, so the rule this line used to state was right only by accident of `-np 1`. Read it off `/props` (`default_generation_settings.n_ctx`) or the startup line's `n_ctx_slot`, never by arithmetic on the flag. **A vision model is two files.** The weights carry the language model and the vision tower is a separate `mmproj-*.gguf` that `--mmproj` must name; without it the server starts, answers well, reports `modalities.vision: false`, and the model tells anyone who sends it a screenshot that it cannot see images — which reads as a limitation of the weights. `scripts/mmproj.sh` now refuses to start without one. MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`). **A transient unit** — now `llama-local.service` (`systemctl --user status llama-local`; it was `llama-qwen` when this was written, and that name no longer resolves), not a tmux pane — see below |
| 8081 | harrier-oss-v1-0.6b | **up, serving embeddings** (`--embeddings --pooling last --embd-normalize 2`). This is where the graph's embeddings come from — they moved off Ollama onto llama-server, so any doc still naming `MECHA_GRAPH_OLLAMA_URL` is stale. One model per process, so this cannot be the chat port as well: pointing both at 8080 sends embedding requests to the chat model. |
| 8083 | Qwen3.8-27B | **down as of 2026-08-20** (was up on 2026-08-16). Nothing in config depends on it, so nothing is broken by it — noted because the previous pass recorded it up and a reader would otherwise assume it still is |
| 8082 | gemma-4-26B-A4B | **down — restart it before any judged run.** The eval judge and nightly validate's judge both point here, so `mecha eval` with a `judge` rubric and the nightly validate will fail without it. `scripts/start-gemma26.sh` |
| 8888 | SearXNG | up (docker, JSON format enabled) — **but every *general* engine was refusing this IP on 2026-08-21**: brave and google cse `Suspended: too many requests`, duckduckgo and startpage `CAPTCHA`, mojeek `access denied`. The specialist engines (lib.rs, crossref, arxiv, openalex, stackoverflow) answer fine. Partially recovered the same afternoon. This is why Exa and Tavily were added — a scraping metasearch loses the anti-bot race, and the answer is a backend contractually entitled to the data, not a better scraper |

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

**Web search is three backends now, and none of it is in any repository.**
As of 2026-08-21 `~/.mecha/config.toml` carries three `[[search]]` blocks in
preference order — `searxng` (`base_url = "http://127.0.0.1:8888"`), `exa`
(`api_key_env = "EXA_API_KEY"`, `prefer_deep = true`), `tavily`
(`api_key_env = "TAVILY_API_KEY"`, no `prefer_deep`, a pure fallback). Ordering
was measured rather than assumed: a quick Exa search bills **$0.007** against
Tavily's $0.008, read off Exa's own `costDollars` in the response, and Exa's
recurring free tier is the larger (~1,430/month against 1,000). The
`contents: {text: …}` block mecha sends adds **nothing** — text extracts are
bundled into a search call, though Exa's price list bills Contents separately
at $1/1k pages and predicts otherwise.

**The keys are in a third place that is also in no repository.**
`~/.config/environment.d/mecha.conf`, mode 0600, holding `EXA_API_KEY` and
`TAVILY_API_KEY`. That path rather than `~/.bashrc` because **only
`environment.d` is read by the systemd user manager** — and unattended search
happens inside `mecha-slack` and `mecha-triggers`, so a key exported from
`.bashrc` works perfectly when tested by hand and is invisible to every run
that matters. `.bashrc` now *sources* that file instead of duplicating the
value, so one edit reaches both; non-interactive shells still miss it, which
is why the units read `environment.d` directly. Verified by reading
`/proc/<pid>/environ` of each service, never by assuming the restart inherited
it. `ANTHROPIC_API_KEY` is still `.bashrc`-only and has therefore never been
visible to any unit — harmless today only because every unit runs
`provider = "local"`.

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
  `--holdout 0.25 --propose` → `rules propose-retirements` → `work clean` →
  `harness ruminate --sessions 16` (added 2026-08-22: the self-improvement
  pass — see that section). Logs land in
  `~/.mecha/learning/logs/<date>.log`; pending proposals wait in
  `mecha proposals`, harness candidates in `mecha harness list`.
  **Confirmed enabled 2026-08-05; unit `ExecStart` re-verified 2026-08-22.**
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

**Swept again 2026-08-25**, and that pass is the reason this section carries
a warning. The mail section below had listed six shipped phases as unbuilt
since 2026-08-19 — see the trap in [`HISTORY.md`](HISTORY.md) under Measuring.
Two further items closed the same day and moved out (the candidate-class gap
and `show_file`'s call-time config read). The blanket sentence that follows
was true of the items it names and was **not** true of the mail section, which
is why a claim of full coverage now needs the evidence beside it.

Every item below was re-verified against source on **2026-08-24** (a
72-item sweep after the day the phone became a terminal — the web-surface
and voice arcs, 45 commits). Almost everything held its verdict; the items
that changed are rewritten in place below, chiefly: the approval race is
now solved once (`serve/present.rs`) with routing to Slack still open, the
`mecha serve` item part-shipped as a third front-end rather than a shared
backend, and the "Slack `ask_user` is structurally absent" claim is half
false since `Asker::ask_in` landed. A new subsection right after the
remote-control one holds what the new code left open. The prior full pass
was **2026-08-20** — MCP
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

### The self-improvement loop — closed 2026-08-22, now producing its own record

`mecha harness ruminate` runs nightly from `ruminate.sh` (after `work clean`):
diagnose one change from the run corpus → persist it as a candidate → measure
it by counterfactual replay of recent sessions → dispose through
`candidate::judge`. A holdout-confirmed config win auto-accepts into the
override layer (`~/.mecha/learning/harness/overrides.toml`, applied beneath
every config file so the user always wins; `mecha harness revert` undoes);
prose and architecture stage for a person; security-class stages with the
standing warning and is never measured. CLAUDE.md "Harness rumination" holds
the design; `docs/SELF-IMPROVEMENT-RESEARCH.md` §13 records the rulings —
auto-accept per §13.3 is Luke's explicit 2026-08-22 instruction, do not
re-ask it. The corpus is live (64 runs of qwen3.6-35b-a3b across 280 sessions
as of 2026-08-22), and the first nightly pass ran the same evening: the
diagnostician declined on a healthy corpus, which is the designed answer.

What is actually open now:

- **Read the record, weekly at first.** `mecha harness list --all` and the
  nightly log. §2's failure mode (harness updating without benefiting) is now
  answerable from the store instead of from impression — but only if someone
  reads it. Doctor flags a candidate staged past 72h; nothing flags a month
  of rejections that should have been declines, which is the pattern worth
  looking for by eye.
- **The content-sensitive arm is still a human spend.** A prose-class
  candidate stages with its `mecha eval --ab-config` command attached and
  waits; nothing runs eval arms unattended, deliberately (a judge-graded arm
  costs real runs and grades what the model *said*, which replay cannot see).
  If prose candidates accumulate, decide then whether that spend gets a
  budget.
- **Divergence rate is unmeasured.** Replay under a pinned seed
  (`seed = 42` on `[providers.local]`) is reproducible, but a
  behaviour-visible change diverges from the recording by design and drops
  the episode. If most episodes drop on real candidates, the session arm
  yields thin evidence and everything routes to review — which is safe, and
  worth knowing before trusting the loop is "measuring". The candidate
  records carry `diverged` lists, so the answer accumulates on its own.
- **Security boundaries stay unproposable** per §13.2 — interlock, path
  jail, sandbox and outbox routing reach a human however anything scores,
  and the standing recommendation is that they are never proposed at all: a
  loop that can argue for widening its own confinement will eventually argue
  well, and the metric agrees with it, because a run that can reach the
  network fails fewer calls.

### The remote control — built, with one path nobody has been able to run

`/remote-control <name>` mirrors a live TUI session into a named Slack thread,
both directions, files included. Built and merged 2026-08-21 (`d266fe8`), live
on this machine. `docs/REMOTE-CONTROL-DESIGN.md` is the design; the arc is in
[`HISTORY.md`](HISTORY.md) under 2026-08-21. What remains:

- **`/send` still reads the whole global config at call time.**
  `slack/send.rs:141` (`send_file`) loads `Config::load_global` for one number,
  and the TUI's `/send` reaches it. Same root cause as the `show_file` bug
  fixed 2026-08-25 and a milder version of it — the parse error lands on a
  keystroke the user just pressed rather than on a tool call two hours in, so
  it is comprehensible where the other was not. Closing it means a field on
  the TUI's `Live` struct, beside `todo` and `skill`, which ride there for the
  same reason: a `/model` switch rebuilds the agent and refreshes them.
  Deliberately left out of the `show_file` fix rather than overlooked.

- **A top-level DM starts a connector run even while a session is attached.**
  Working as designed — the thread is the unit — but the observed failure mode
  is a person posting a screenshot into the DM rather than the attached
  thread, and getting a fresh connector conversation in a different workspace
  with no sign that the live session exists. The cheap fix is a line in the
  connector's reply naming the live attach; routing it is a bigger decision
  that §2 of the design already settled the other way.

- **Approvals cannot be answered from the mirrored thread — but the race is
  now solved once, elsewhere.** The atomic claim design §15 asked for shipped
  in `serve/present.rs` (2026-08-24): `Questions::open`/`answer` — a second
  device answering an already-claimed card gets `false`, timeouts resolve as
  `Blocked`, cancel drains parked cards. What remains here is routing: the
  TUI still says *waiting for you at the terminal* (`tui/mod.rs:1127`), and
  wiring the mirrored thread means a Slack renderer over the same
  `Questions`, not a second claim mechanism.
- **`mecha serve` shipped 2026-08-24 — as a third front-end, not yet the
  shared backend.** The hosted process exists, survives SSH drops, reaches
  a second machine (the phone, daily), and the voice facade mounts inside
  it. What the item originally promised and remains open: the TUI
  (`tui/mod.rs:943`) and the Slack connector still build their own `Agent`
  each — "both front-ends are thin clients of it" is not true, and deciding
  whether it ever should be is now a real question rather than a design
  sketch, because three agent-owning processes on one llama-server is the
  live shape. Design §15; `REMOTE-SURFACE-DESIGN.md` is the authority for
  what serve itself still owes.

### The phone surface and voice — what is still open

Everything here is verified in source, re-checked 2026-08-25; the arcs' own
docs (`REMOTE-SURFACE-DESIGN.md`, `VOICE-RESEARCH.md` §7) hold the shipped
half.

D3 shipped 2026-08-25 — a call is the chat session it was started from.
The narrative is in HISTORY under that date; `VOICE-RESEARCH.md` §7 holds
the mechanism and every decision. What it left standing:

- **A `voice:` session is now the fallback, not the norm.** A call from the
  app is a `web:` session, so the drawer's `voice` badge marks only calls
  that named nothing — a direct offer to the worker, or a key no front-end
  held. The badge's meaning quietly changed from "this was spoken" to "this
  call had no page behind it"; the per-turn `spoken` marker in the
  transcript is what carries the first meaning now. Worth a deliberate
  decision about what the drawer should say, rather than leaving two
  meanings one word apart.
- **The page's mode chip does not describe spoken turns.** `--voice-yes`
  travels with the *turn* (`TurnOpts::approve_all`,
  `serve/chat.rs`), so a spoken turn runs at Allow while a typed turn in
  the same conversation runs at whatever the chip says. Luke's call,
  2026-08-25, and defensible because nothing structural moved — the
  interlock is ahead of the approver, sends still stage, and taint
  accumulates across both doors. Still worth revisiting: a label true of
  one door and not the other is the shape this project usually refuses,
  and if voice ever grows spoken approvals, that flag is what they replace.
- **`mecha serve` never drains its chat runs on shutdown**, and since D3
  that gap covers spoken turns too. Not a regression in practice —
  `axum::serve` is not wrapped in `with_graceful_shutdown` and there is no
  SIGTERM handler, so systemd's stop is a hard kill and `facade.shutdown()`
  was already unreachable there. But the standalone `mecha voice-serve`
  *does* handle SIGTERM and the mounted one does not, which is now the only
  place that difference shows. Closing it means deciding who owns SIGTERM
  in a process holding SSE streams and pending approval cards.
- **A second device watching a typed send sees the reply and not the
  prompt.** A spoken turn broadcasts `WireEvent::User` because it has no
  local echo anywhere; a typed send is still echoed only by the page that
  typed it, so broadcasting it too would render it twice there. Ending it
  properly means the page distinguishing its own echo from the broadcast,
  which is a small change nobody has needed yet.
- **The owner's first-day feedback backlog is `REMOTE-SURFACE-DESIGN.md`
  §12** — chat model switching, a plain mail inbox + compose, notes/tasks
  voice capture and listings, the task→agent handoff (the big one), Home
  navigation/widgets. Triaged there, not restated here. The plain inbox +
  compose and the notes/tasks voice capture shipped 2026-08-24 night (Arcs B
  and C); the rest stands.
- **Delegation is a conversation as of 2026-08-26 (eighth pass)**, which
  closes the phone surface's biggest gap and opens two smaller ones.
  `ask mecha` opens the task's chat session — voice, uploads, todo panel,
  approval cards, steering by typing — the model speaks first, the board does
  not move, and *let it carry on without me* hands the same transcript to a
  detached child. HISTORY has the narrative and `CLAUDE.md` the invariants.
  What that leaves open:
  - **Admission control (R1 / phase 6) shipped 2026-08-26**
    (`mecha-core/src/permit.rs:50`), after R3 was run: three background seats
    against the server's four, interactive work uncounted. What is left of it
    is the half that was never the point — **there is still no queue.** A
    refused delegation is told to try again; nothing remembers it and nothing
    starts it when a seat frees. That is deliberate for now (a queue that
    starts work unattended is a scheduler, and the trigger daemon is already
    the place for scheduled runs), but it is the gap between what shipped and
    what the owner asked for — *"when the question is answered it should
    resume in the queue"*. Worth building only if refusals actually happen;
    the permit files make that countable.
  - **PR #84 is open and unmerged**, with the last three findings from the
    #78 review: the pressure line stating a cost and a turn count derived from
    different numbers (a 400-token pace read as *"~1k each, so about 224
    more"* against a 100k limit), an unreachable match arm beside it, and
    `replay.rs` / `harness_probe.rs` building `ToolCtx::default()` so a
    recorded compaction answers *"compaction is not enabled for this run"* —
    false of the run being replayed, a divergence under
    `--on-divergence=live`, and in a harness probe a `compact_at_tokens`
    candidate measured on runs that never compacted. 1,566 tests and clean
    lint locally; **CI unverified** — it had not reported when the session
    ended. Check it before merging rather than trusting this line.
  - **The overnight half of R3 is still unmeasured.** The contention half was
    run (see the measurement record) and answered the question R1 rested on.
    What §3.4 also asks — *does a conversation parked overnight get its prefix
    back, or has a night of triggers and chat pushed it out?* — has not been.
    The contention result makes it more likely to be fine than the design
    assumed, which is a reason to expect a cheap answer, not a reason to skip
    it. `scripts/slot-contention.py` is the shape; the missing arm is time,
    not concurrency.
  - **`mecha serve` still has no graceful shutdown**, and it now costs more
    than it did: a restart kills a conversation-owned run mid-flight, where a
    handed-over one survives. The conversation itself survives either way
    (the transcript is the record), so what is lost is the partial turn. Same
    unresolved question as before — who owns SIGTERM in a process holding SSE
    streams and pending approval cards.
- **The task→agent handoff is built through phase 4** —
  `docs/TASK-AGENT-DESIGN.md` is its authority and HISTORY has what shipped.
  The return path and D16's card states closed on 2026-08-26 (second pass);
  B1 and B2 closed the same evening, both **amended first** — re-reading
  them against the shipped row changed both, and the amendments sit beside
  the originals in the design doc. **D4 shipped 2026-08-26 (sixth pass) —
  the seed points at context rather than assembling it into the prompt; the
  amendment sits beside D4 in the design doc and HISTORY has the narrative.
  What is left of the arc is one measurement and one deferred decision.**
  - **Whether runs follow the pointer, which is D4's own acceptance test.**
    *"Measured in Phase 4, not assumed in Phase 1"* has two runs behind it,
    which is a direction rather than a result. Both followed. A task naming
    only a project drew seven graph calls (`kg_search` ×4, `kg_entity` ×3);
    a task captured from a real three-message mail thread called
    `mail__mail_get_thread` **as its first act**, before anything else, and
    the recorded taint came back `private + untrusted` — the bytes arriving
    as a tool result the interlock accounted for, which is the whole reason
    the seed carries a pointer. Keep watching rather than concluding: the
    query is a scan of task-titled transcripts for a call to the tool the
    seed named, and pasting the context stays available if runs stop
    following it.
  - **A captured task's *name* is untrusted text, and the seed cannot fix
    it.** D4 withholds the pointer's `label` because a subject line is prose
    somebody else composed — but `mail task` defaults the task **name** to
    the classifier's `one_line`, falling back to the raw subject
    (`commands/mail.rs:1947`), so a default capture puts a model's paraphrase
    of a stranger's mail at the top of a privileged run's opening
    instruction. The front door's own rule is that *a paraphrase of an
    injection is the injection rearranged*, and this is that rule's own
    counterexample sitting one store over. Found by the PR reviewer on #68,
    not by the tests — which passed because the fixture's name and label
    differ, and in production they usually will not.
    **Not fixable in the seed**: the name is what the task *is*, and a run
    that cannot see it cannot work it. So the question is what capture should
    default to, and the options are worth pricing before one is picked —
    require `--name` and refuse to default (safest, and friction on the verb
    that exists to be cheap), keep the default but mark a derived name on the
    record so the seed can say where it came from, or leave it and accept
    that a delegated run's subject line is attacker-influenced. Pre-existing
    since capture shipped 2026-08-26; nothing regressed, and it is written
    down here because `CLAUDE.md` previously read as though the boundary held.
  - **The seed's bullet order is now pinned by a test, and the reason it
    moved did not survive its own measurement.** The pointer bullets first
    shipped directly beneath *"do not ask what you can find out"*, so the
    section ended on two consecutive reasons not to ask; pooling every
    substantive run of the day made that look like a regression —
    `ask_user` in 5 of 6 runs under the previous seed against 0 of 4 under
    the new one — and the bullets were moved above the asking block on that
    reading. **The pooled number is confounded by task and should not be
    quoted.** The arms ran different tasks; the only within-task series is
    one board item (*decide whether to submit to a conference*, six runs),
    and there `ask_user` fired once in two old-seed runs, zero in two
    ask-first runs, and zero in two lookup-first runs. So the order change
    is justified on reading order — here is what you can find out, *then*
    ask about what is left — and **not** on a measured effect, and the
    question of whether this model asks less under the new seed is still
    open at n far too small. The test asserts the order rather than the
    outcome, which is the part that is actually decided.
  - **Front-loading landed as prose, not as a parked question, on the one
    run that had questions to ask.** The mail run ended `completed` with six
    numbered decisions in its final answer and **zero `ask_user` calls**,
    with the tool on its surface (63 tools, `ask_user` among them,
    `kg_task_update` correctly withheld). So the questions exist only inside
    a session transcript: `mecha questions list` does not show them, the
    phone's card cannot offer them, and the delegation reads as finished
    rather than as blocked — which is the state D13 exists to make
    impossible. One run, and the mechanism does fire elsewhere (two
    questions are parked from other tasks), so this is a prompt-adherence
    observation and not a broken store. It is also the exact seam D12 was
    decided against on: the seed asks on the user turn because that is the
    channel this model obeys, and here it obeyed the *content* of the
    instruction (ask first, one question, list every unknown) while ignoring
    its *mechanism*. Worth a second look before concluding anything — if it
    repeats, the cheap fix is naming the tool call in the sentence that
    already says what to ask.
  - **D12, the plan gate — decided against as written, on 2026-08-26, and
    the cheap half shipped instead.** Do not build it from the design doc;
    read `work_prompt`'s doc comment in `commands/tasks.rs` first, which
    carries the reasoning. In short: D12 made the *todo list* the
    human-editable object, which conflates the agent's execution ledger with
    a reviewable plan — every other system keeps those apart, and `todo.rs`
    already forbids a second author of its state. Three findings decided it:
    `VERIFICATION-RESEARCH.md` argues against plan-first on this hardware in
    particular (small models *collapse* under plan-and-execute; a bad plan
    measures worse than no plan); the gate's trigger rested on a `todo` write
    this model was measured not to make from prompting (HISTORY 2026-08-04,
    zero calls in 20 runs); and D12's own cited evidence — Copilot's
    38.1% → 69% — was about tuning the *seed*, which is `work_prompt`.
    So the seed now front-loads questions, with a guard against the opposite
    failure, delivered on the user turn because that is the one channel the
    probe found this model obeys.

    **What is still unbuilt is the part front-loading cannot reach**:
    misalignment the model does not notice, where a confidently wrong plan
    asks nothing. The decision to build a reviewable plan *document* —
    separate from the todos, on superpowers'/Claude Code's split — is
    deferred until the corpus argues for it, and the query is now available:
    **delegations that ended `ready for review` and were then dropped or
    reworked rather than marked done.** Board status transitions plus
    `RunStats` plus the session answer it. If that number is small the gate
    is friction; if it is large, the same query names the examples it would
    have caught. The asymmetry that would force the question sooner: a
    web-launched delegation is `--unattended` today, so it can only read and
    stage, and the downside of letting a run go is bounded by construction.
    That stops being true the moment a delegated run can get approval from a
    present human.
  - **Phase 6** is admission control (R1) — explicitly not worth building
    until more than one delegation at a time is routine.
  Two things found while building the return path are worth knowing before
  touching this arc again. **A task run's `Record::Outcome` only exists from
  2026-08-26 (second pass) onward** — `tasks work` and `questions answer`
  were the two front-ends that never wrote one, so every delegation before
  that date is invisible to `runlog` and to `sessions health`, and the
  design doc's own "task shape" question ("`RunStats` already records enough
  to answer it later") was false for exactly the runs it was about. The
  corpus starts accumulating now. And **`/api/tasks` passes `--closed`**,
  because the drawer's `done` view filters on `done | dropped` and the list
  it filtered had never contained either — a view that had been
  structurally empty since it shipped, in the way that reads as "you have
  finished nothing" rather than as a filter that cannot match.

  Part 3 of that document records a refusal worth knowing before it is
  proposed a third time: **no KV-offload manager** — `-cram` already is one,
  and the hand-rolled `--cache-idle-slots` was measured costing 20.5s
  re-prefills and removed.
- **`--voice-yes` is a deliberate posture with a named risk.** Voice runs
  get `ModeApprover { Allow }` (`voice/mod.rs:552`; the unit runs the flag)
  while web chat defaults read-only with live cards — so one process serves
  two dialects at two postures, and the hands-free one is the permissive
  one. The reasoning (a voice call has a present owner and cards cannot be
  tapped mid-sentence) is recorded in the flag's doc; if voice ever grows
  spoken approvals, this flag is what they replace. `allow` mode is equally
  deliberately **not** offered from the page (`serve/chat.rs`) — approve one
  call at a time in `ask` mode instead.
- **The web app imports outside its package root** —
  `../../../scripts/voice/voice-core.js` — which couples `npm run build` to
  the whole checkout's layout. It was deliberate when the module served two
  shells and had to be owned by neither; since the standalone page was
  retired (2026-08-25) the app is the only consumer, so the argument now is
  only that the module must stay framework-free for whatever embeds voice
  next. Moving it under `web/src/lib/` would be defensible and would end the
  outside-the-root import; it has not been done because the module is the
  voice arc's to own and the web app's build is not.
- **A phone verdict has no undo**, which doubles the stakes of the
  `undecide <seed>` design below: `POST /api/queue/verdict` is a second
  irreversible verdict surface — and since the global similarity layer
  (2026-08-24 evening), one tap can now carry a cross-class cascade of
  dozens, so the undo design is worth pulling forward. **Unchanged by the
  2026-08-25 pass**, which only made a *failed* verdict honest: the card now
  stays and offers the two ways through (`bind`, accept-as-new-topic) instead
  of vanishing while the candidate stayed pending. A verdict that lands is as
  irreversible as it ever was, and accept-as-new-topic adds a tap that mints
  a topic node.
- **PWA install and Web Push are unstarted** (design Phase 5). Until push,
  Slack remains the nudge channel for staged drafts — D11 keeps it anyway.
- **A draft released by voice leaves a stale card on the page.** Since
  2026-08-25 both doors offer the same drafts: the page draws a confirm card
  and the call asks about it aloud, deliberately, because they are two views
  of one conversation. Whichever one acts, the other's copy does not know.
  The store refuses the second attempt — `outbox approve` on a resolved item
  errors and the card shows the CLI's own words — so nothing double-sends;
  what is missing is the *notification*, which means a `SessionHost` method
  the facade can call to broadcast a `WireEvent` into the page's stream. Not
  built because it is the only thing the trait would carry and no one has
  been bitten yet.
- **The spoken confirmation is not recorded in the conversation.** A "yes"
  handled by the harness runs no model turn — 0 prompt tokens, 0 completion
  — so the transcript has no record of the exchange, and a later turn cannot
  see that the send was authorised. The *authorisation itself* is on the
  record where it belongs (the outbox item's status and `resolved_at`), so
  this is fidelity rather than an audit hole: `distill` and the run-quality
  corpus simply do not see it. Closing it is the same `SessionHost` seam as
  the item above.
- **The model still narrates a staged draft the harness is about to read
  back.** D10 now tells it to say one short clause and stop, and the local
  model mostly complies ("Drafted and waiting for release."), but not
  reliably — the run that opened the arc said the whole thing twice. The
  duplication is verbose rather than harmful, and arguably a feature: the
  harness readback is the authoritative one, so a *disagreement* between the
  two is audible. Worth a decision rather than more prompt tuning.
- **The real calls happened on 2026-08-25, and what they found is now
  fixed — but three questions the item was really about are still open.**
  Luke made several calls from the phone (`af_heart` voice, a calendar
  question round-tripped, one that ran 5.5 minutes) and reported one thing:
  calls ending by themselves. That had two causes, both closed the same day
  and both in HISTORY — pipecat's 300-second idle timeout, and ICE
  `disconnected` treated as terminal. What the calls did **not** settle,
  because nobody was listening for it, is what the item was written to ask:
  whether the AEC/echo filter holds on a speakerphone walk, which of the
  seven voices is worth keeping (six Kokoro-derived references plus
  Chatterbox's own `default`), and whether the D10 block re-arriving after
  every typed turn keeps replies ear-shaped or is one copy too few. Those
  need a call taken *for* them.
- **One call connected deaf and nobody knows why.** 2026-08-25 13:27:43: the
  peer connection established, the data channel opened, the voice config
  round-tripped — and `Timeout: No audio frame received` repeated every two
  seconds for the whole 21 seconds, from the first one. The retry 8 seconds
  later worked normally. One occurrence, not reproduced, and distinct from
  the teardown bug that *was* fixed. Worth watching for: a call that connects
  and hears nothing is indistinguishable from a call the model is ignoring,
  and the client has no signal for it at all.
- **Voice input has no denoising, and the research says leave it that way —
  but the VAD is untuned, which is the actual lever.** Deferred 2026-08-24
  with the finding attached, because the obvious fix is the wrong one.
  Denoising and false-triggering are different jobs: the VAD decides what
  becomes a turn, the denoiser only cleans what already got through. And
  Pipecat's filter mutates the frame in place
  (`BaseInputTransport`: `frame.audio = await ...filter(frame.audio)`), so
  `audio_in_filter=RNNoiseFilter()` denoises the audio reaching **Parakeet**
  too — and a systematic study that tested speech enhancement against four
  ASR systems *including NVIDIA Parakeet* found it degraded every one, by
  1.1–46.6% absolute semWER (arXiv 2512.17562; at 10 dB SNR, 8.82% → 25.83%),
  through artifacts inaudible to humans and the removal of features the model
  was already using. What you would want is denoise-for-VAD, raw-for-STT, and
  Pipecat's filter API cannot express it.

  **Step (1) shipped 2026-08-25 (`c7fc975`), and not by tuning.** The VAD was
  dropped from the turn-*start* strategies
  (`worker.py:347`, `start=[TranscriptionUserTurnStartStrategy(...)]`), so a
  wordless segment now emits no `TranscriptionFrame` at all, reaches no
  strategy, and the bot simply keeps talking — "resume on an empty
  transcript" achieved by never stopping, which needs no state to unwind.
  The analyser stays because it still *segments*; `start_secs` moved 0.2 →
  0.3 (`worker.py:346`) and `confidence`/`min_volume` were deliberately left
  alone, since the owner's measured speech is ~0.024 RMS against a 0.14
  tuning assumption and raising thresholds to chase noise would start
  dropping quiet real speech. The cost, stated in the source: Parakeet is
  offline, so barge-in is "finish the phrase and it stops" rather than
  instant.

  **What is still open is (2) and the denoiser question.**
  `MIN_SEGMENT_RMS = 0.010` (`worker.py:75`) sits **one thousandth**
  above the measured room noise of 0.009 — tuned in a quiet room, and wind
  will not notice it; re-measure it in the conditions that matter before
  trusting it outdoors. Note the *cost* of a false positive already collapsed
  twice over: with the Parakeet swap a noise segment yields empty or garbage
  text the gates drop where Voxtral produced a confident fabricated answer
  attributed to the owner, and since (1) it no longer interrupts either.
  Only after (2): RNNoise is one
  `pip install pipecat-ai[rnnoise]` away, but wind is non-stationary
  broadband — RNNoise's weakest case and DeepFilterNet3's strength, and DFN
  is not in Pipecat core (issue #3266). Measure Parakeet's WER either way.
- **Parakeet hotword biasing is blocked on a missing `bpe.vocab`, and the
  failure mode is a segfault.** Deferred 2026-08-24. This is the wanted half
  of what an LLM-decoder ASR would give (domain vocabulary — contacts,
  project and place names from the graph) without the half that lost Voxtral
  the seat, because a hotword list is a decoding-time constraint on the
  search and not a prompt: there is no channel down which an instruction can
  travel. `sherpa_onnx` 1.13.6 exposes `hotwords_file` / `hotwords_score` /
  `modeling_unit` / `bpe_vocab`, and `create_stream(hotwords=...)` takes them
  per stream, so a list could be refreshed nightly without reloading the
  model. But the model directory
  (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`) ships `tokens.txt` — 8,193
  `▁`-prefixed sentencepiece tokens — and **no `bpe.vocab`**. Measured: the
  `cjkchar` default does not crash and silently does nothing on a BPE model
  (output identical to baseline); `modeling_unit="bpe"` **segfaults inside
  `create_stream()`**, and a `bpe.vocab` reconstructed from `tokens.txt`
  builds the recognizer and then segfaults in the same place. Needs either a
  correct vocab or a model package that ships one. **The segfault is itself
  the finding**: sherpa-onnx crashes the process rather than raising, so any
  real version must validate a generated list in a subprocess before the
  live recognizer sees it — otherwise a bad nightly write takes `:8992` down
  and voice goes deaf with no error text anywhere.

### The goal system — rungs 0–7's observation half and the quarantined appraiser shipped; the model half of step appraisal is what's left

`docs/GOAL-SYSTEM-DESIGN.md` is the authority and carries a status header
saying which rungs shipped. The arc's premise, which is what makes the rest
follow: **every evaluative signal mecha had was a cost or a correction**, so a
run could be recorded as having gone badly and never as having gone well.

Rungs 0–6, rung 7's observation half, and rung 7's quarantined appraiser
(`--appraise`, below) are in `main`. Everything with no model, no charter and
no adversary in it is now built, which is §14's ordering and not a
coincidence — and the appraiser is the first thing here that actually spends
a model call, on the observation-first shape every other rung took: offline,
budgeted, and left without a store until a real corpus says it earns one.

**PR-stack status, verified 2026-08-27T22:15Z against `main` at `a0638c8`.**
The whole appraisal/learning-UI/validation/task-seed stack that was in flight
when the paragraph below was first written is now merged, nine PRs in the
order they landed: **#86** (`bcd7d4f`), **#87** (`c81d0fb`), **#93**
(`4f28221` — carries forward #86's fourth-round fix, which had been pushed
eight minutes after #86 merged on the round-3 tip and so never reached
`main`; no open findings), **#88** (`18a6fcf4`), **#89** (`1e04c114`), **#90**
(`c630ff94`), **#91** (`f0ca8ca`, mecha-80's probe work), **#94** (`2f432b3`,
see below) and **#92** (`a0638c8`, mecha-4c's task-seed fix — its content is
already folded into the rung-7 section below, in that PR's own words, so it
is not repeated here). Every review finding this section once listed as open
against #86, #88, #89, #90 and #93 is fixed on `main` today, verified by
reading the merged source rather than trusting a review thread — see
HISTORY's 2026-08-27 (third pass) for the file:line record of #86/#88/#90/#93,
and the paragraph below for #89.

**CI on #89's own merge commit shows `cancelled`, not `success`** — checked
per this file's own rule against `gh api repos/ljchang/mecha/commits/1e04c1143afee84e3bb57c023a75d80f772e9e3a/check-runs`,
because #90 merged twelve seconds later and superseded it on the same ref
before its run finished. `main`'s actual tip carries every PR's content and
its own check-runs are all `success`, so nothing is unverified today — but a
reader who checks out `1e04c114` in isolation, rather than `main`'s HEAD,
would find a commit CI never finished grading.

**#89 shipped with real findings nobody circled back to fix, and #94 is the
fix.** Two of #89's own review rounds flagged problems in the new `/learning`
modal that a later round did not re-raise, and a first pass at cleanup — this
document's own earlier draft — found four of them still true on `main` after
#89 merged. Re-reading *every* review comment on #89, not just the ones that
first pass covered, turned up two more of the same shape. All six are fixed
in **#94** (`2f432b3`), verified against the current tree rather than the
PR's own account of itself:

- `App::a_modal_is_up` now checks `self.learning.is_some()`
  (`mecha-cli/src/tui/mod.rs:601`), so the modal's mouse capture releases
  like every sibling's.
- `reject` on a proposal now passes `--reason "rejected from /learning"`,
  matching the exact fixed-string pattern `/queues` already used for the same
  command rather than inventing a new one (`mecha-cli/src/tui/mod.rs:6274`).
- `doctor`'s starved-learner counter now skips a row with `dropped_at` set
  when counting `excluded` (`mecha-core/src/doctor.rs:1442`), and `mecha
  learn` reports its own `dropped_by_owner` count alongside "excluded by
  origin" instead of folding the two together
  (`mecha-cli/src/commands/learn.rs:107-150`).
- `learning_act` no longer calls `self_cli` synchronously from the key
  handler; every verb now runs through the detached-and-watched shape
  `/outbox` and `/triggers` already use, polled by a new `Watch::Learning`
  (`mecha-cli/src/tui/mod.rs:6140-6161`), so a `/learning` mutation made while
  `reflect`/`learn` holds the store's flock across a model call no longer
  freezes the whole TUI.
- `LearningStore::reflexion` carries the same empty-needle guard
  `rules.rs::find_rule` got in #89's own first review pass, which had never
  been applied to this sibling lookup (`mecha-core/src/learning.rs:1103-1110`).
- `/queues`' `move_sel` now clears `review_detail`/`detail_scroll`
  (`mecha-cli/src/tui/queues.rs:500-508`) — before this, `j`/`k` while a
  proposal's fetched text was open could move the cursor onto the next
  proposal while the stale text of the previous one stayed on screen, so an
  `a`/`r` right after could accept or reject something never actually read.

The general lesson from the first pass missing four of these stands
regardless of how quickly the fix landed: **a later review round approving a
PR is not evidence that an earlier round's findings were addressed** — a
reviewer re-reads the current diff, not the accumulated list of everything
anyone has said about it. See HISTORY's 2026-08-27 (third pass) for the
fuller argument.

**Open now:**

- **Rung 7's measurement came back, and it decides what to build next.**
  `mecha sessions appraise` over the live store: 459 sessions read, **120
  appraised, 119 signed goal errors recorded across two channels, and 100% of
  the labels neutral** (2026-08-27). Eleven of the errors are positive — a
  draft the owner read and sent unchanged — which is the one signal in this
  system that says something went well and had never been counted anywhere.

  Nothing is broken: every label that could have fired needs a dimension
  nothing measures, and `appraisal.rs` names which rung buys which. What the
  corpus adds to that table is a **build order**. A counterfactual verdict is
  what gives an intervention error a label at all, and interventions are 102 of
  the 119 (**but see reach, below — the operative figure is 13**); the charter
  — §14's rung 10 — buys only the eleven positive ones. So
  if the readout is the goal, the probe is the cheaper half and should come
  first. That is a change to §14's order, argued from a measurement rather than
  from the design.

  **Three things narrow that, found on 2026-08-27 by building the probe. Read
  them before acting on the paragraph above.** They are separate quantities and
  were fused in the original claim:

  1. **Reach.** A probe can only address a `steer` or a `denial` — a
     `followup` is a later user turn, and removing it does not leave a run that
     would have got there anyway, which is why `validate` reaches followups
     with a judge instead. Most interventions are followups, so the probe
     addresses **13 of the 102**, not all of them. A structural ceiling, not a
     budget.
  2. **Yield.** Of those 13, **12 come back inconclusive** — the replay
     departs before reaching the intervention. See the validation section
     below; the cause is found and the fix is in, but it recovers nothing
     already recorded.
  3. **Neither branch of the comparison was ever measured.** The charter's
     eleven positive errors do not become `Pride` just because a charter
     exists — `Pride` is *against a charter line*, and **no session in the
     store names a goal at all**. `GoalError.goal` is always `None`, so
     frustration is structurally unreachable too. The reordering may still be
     right; it was argued from one number that was never checked and one that
     was wrong.

- **`serves:` has never carried a value in production, including in the runs it
  was built for.** 112 of 120 appraised sessions wrote a plan; **0 named what
  it serves** — and 15 of them are delegated task runs (`task-d063fe34` ×11 and
  two others), which is the case §2's *one list, one task* argument is about.
  The field is in the `todo` schema marked "Optional", and nothing in a
  delegated run's seed tells the model the task uid or asks for one. The seed
  already builds its prompt from the board record, so this is a sentence in
  `tasks.rs` rather than a design question — and until it lands, two of the
  eleven affect labels are unreachable for a reason that is neither the probe
  nor the charter.

  **Fixed 2026-08-27, and the diagnosis is narrower than the paragraph above
  states.** `TodoTool::description` already told the model to pass `serves`
  when work serves a task — all 15 delegated runs above carried that
  instruction *and* this task's own id on the seed's `Id:` line, and still
  wrote nothing. So the gap was not "nobody was ever told"; it was that
  nothing bound the schema's generic reminder to *this run's specific* id.
  Both delegated postures (`work_prompt`, `discuss_prompt` in `tasks.rs`) now
  do that binding explicitly, using the id already in the seed — nothing new
  is fetched or paraphrased. **Caught on review** (#92): the first cut of
  this sentence repeated the "nobody was told" framing the code comment and
  CHANGELOG entry originally used, which the schema text already
  contradicted.

  **Whether the narrower fix moves the number is unmeasured, and the 120
  sessions already appraised stay at 0 regardless** — this changes what
  future delegated runs write, not what past ones did, so the next honest
  read of `serves:` coverage needs sessions recorded after this lands, same
  as the surface fix below.

  **And `sessions appraise` reported that zero by construction**, which is the
  sharper half: it passed `&[]` for goals unconditionally, so the command built
  to measure whether the labels were degenerate could never have reported
  anything but zero goals. Absent and zero conflated inside the instrument.
  **Fixed and confirmed in `main` as of `acceaea` (part of #88, merged
  2026-08-27T19:47:23Z)** — verified by reading `commands/sessions.rs` at
  the current tip rather than trusting an earlier claim about it: `appraise`
  now calls `TodoTool::plan_from_transcript` on the loaded messages and
  passes the real goal through. This superseded an earlier, narrower fix
  (`plan_from_transcript` on `feat/appraisal-probe` at `efaa4b9`) written
  independently by the same review pass that fixed the `Frustration`
  mislabelling bug below — the two landed together because both touch
  `of_session`'s goal handling. **#91 has since merged too** (`f0ca8ca`,
  2026-08-27T20:41:57Z) — `efaa4b9` is an ancestor of `main` now — but its
  `named_a_goal` counter and `· 0 named a goal` print line did not survive:
  confirmed absent from the current tree by grep, same as before #91
  merged. Presumably dropped resolving overlap with #88's independent fix,
  since both touch the same call site. If that measurement matters, it
  still needs building from scratch against the current code, not assumed
  to already exist because the commit that once added it is now merged.

  **A second, more consequential bug was fixed in the same pass, and already
  landed** (`acceaea`, #88's review — this is a record of what shipped, not
  an open finding). `of_session` had cloned `goals.first()` onto every error,
  so `Frustration`'s actual definition — *repeated negative error, one goal,
  self-agency* — degenerated to *any two negative errors, once a session has
  a goal at all*, and the check ran before agency/exposure and could mask a
  more informative label (a turn-ceiling failure alongside an embarrassing
  rewritten draft would have reported `Frustration` and hidden both facts).
  The fix is in `affect_of` now: the `repeated` check is filtered to
  `Agency::Own` and grouped by `error_kind` (`Channel` plus, for a
  `Cite::Counter`, the counter's own name), which restores the real
  definition and keeps the rank comparison from burying an exposed error.
  Recorded here because it changes what "the labels resolve now" will
  actually mean the next time `sessions appraise` runs with goals present —
  which is what this PR's own `serves:` seed fix starts making possible —
  and because it was latent until something populated goals widely enough to
  reach it, so a reader might otherwise assume it is still open. It is not;
  check `affect_of` before re-deriving it.

  **Two corrections to that, from the lane building the probe** (`mecha-80`,
  2026-08-27; its probe-specific work is **#91**, merged (`f0ca8ca`) — the
  counts below are theirs and are not independently reproduced here):

  - **The probe does not reach all 102.** Most interventions are `followup` —
    a later user turn rather than text riding with tool results — and there is
    no counterfactual to drive, because removing a later turn does not leave a
    run that would have got there anyway. That is why `validate` reaches
    followups with a judge instead. A structural ceiling, not a budget, and it
    shrinks the probe's share of the readout accordingly.
  - **And today the reachable ones cannot run either** — see the next item,
    which is the larger finding.

- **`mecha validate`'s steer and denial probes have never been able to run on
  an interactive session**, and that is why `validations.jsonl` does not exist.
  `replay_run::replay_registry` bails on any recorded tool name the live
  registry lacks, and `ask_user` is registered *only* by a front-end that owns
  a human — `setup::prepare`'s `interactive` flag picks the approver and does
  not register it (`setup.rs`, the `TerminalApprover` branch). The coupling
  closes: a probe needs an intervention, interventions happen in interactive
  sessions, interactive sessions carry `ask_user`, and the replay refuses to
  build. **Verified against the store: 246 of 408 sessions with a recorded
  tool list carry `ask_user`.** The nightly has been running `validate` and
  every steer/denial probe has skipped, which its `skipped` counter reports and
  nothing reads as *this whole class is unreachable*.

  The contained fix is a **spec-only stub under `OnDivergence::Stop`**: nothing
  executes in that mode, so the registry exists to reproduce the surface the
  model *saw*, and a stub that errors if actually called restores that without
  changing any executed behaviour. It must stay conditional on the mode —
  under the others tools really do run, and there the bail is correct.

  **This paragraph is stale — the stub is built and both callers already use
  it, verified 2026-08-27 by reading `probe.rs` on `main` rather than trusting
  this note.** `probe::drive_arm` — the one function both `mecha validate`
  (via `prepare_probe`) and `mecha sessions appraise --probe` (via
  `prepare_probe_at`) call to actually run an arm — builds its registry with
  `Some(&crate::setup::surface_only_registry())` as the `ask_user`-shaped
  fallback, under `OnDivergence::Stop`, exactly as described above. Landed as
  part of #90/#91. What is still unmeasured is not whether the fix exists but
  whether it *changes the numbers* — nobody has re-run `mecha validate` since
  it landed to confirm `validations.jsonl` now gets written for a steer/denial
  probe that used to silently skip.

  **Do not build rungs 8–10 on the label yet.** §8's prioritised replay and
  §10's memory salience both key off affect, and today affect is a constant.

- **The quarantined appraiser (§5.1) shipped 2026-08-27** — no tools, no
  conversation, typed output, offline via `mecha sessions appraise --appraise`
  (`mecha-core/src/appraisal.rs`'s `AppraiserEvidence`/`appraise_with_model`/
  `apply_appraiser`, `mecha-cli/src/appraiser_pass.rs` for the budget and
  tally, same shape as `--probe`). It reads one already-built `Appraisal`'s
  numbers only — never the transcript, an intervention's text, or a draft's
  body — and returns one more signed `GoalError` (`Channel::Appraisal`,
  `Cite::Appraiser`) or "nothing further", which is the ordinary, correct
  answer and is tallied apart from a budget running out. Smoke tested live
  against 3 real sessions (all came back "nothing further" — too small a
  sample to be the corpus measurement rung 7's observation half already took
  at 120 sessions); running `--appraise` at that scale is the natural next
  step, and it is what decides whether the appraisal **store** now earns its
  place, per this rung's own build note in `GOAL-SYSTEM-DESIGN.md`.

  **What's left of rung 7 is the model half of step appraisal** — the
  escalation, not the common path (see the next item). It costs a model run
  too, same as the appraiser; until either lands with a store behind it the
  record stays derived on the spot, which is `runlog`'s rule and a correction
  to §10.

- **Rung 6 shipped with two gaps that are stated reasons, not sequencing.**
  Both are named in `GOAL-SYSTEM-DESIGN.md` §14 rung 6 and in the modules
  themselves, so neither is rediscovered as an oversight.
  - **Step appraisal reads two of §5.5's five signals** (`step.rs`). The two
    shipped are facts about the span — nothing was attempted, and the last
    attempt did not succeed. Of the three absent, one turned out to belong to
    boredom and is built there; the other two — a span far longer than its
    siblings, a verify-shaped call that passed — each need a threshold nobody
    has measured here or a guess about what a call meant, and the escalation
    that would settle an ambiguous span is rung 7's.
  - **Boredom's rung 2 — consult — is unbuilt** (`boredom.rs`), because
    neither half can be reached: a §7.4 marker does not exist at all, and
    while a skill does, nothing in the `Tool` trait identifies the tool that
    loads one. `narrows_surface_to` answers `None` until a skill is already
    loaded, so it recognises the state the notice exists to escape only after
    the escape has been taken. `Tool::runs_a_fresh_conversation` — added for
    rung 3, fourth in the family with `carried_state`, `fixed_workspace` and
    `narrows_surface_to` — is the shape closing it would take.

- **Nothing has measured rung 6 yet, and the sensor is now there to do it.**
  `RunStats::boredom_notices` and `Corpus::boredom_rate` landed with it and
  `mecha sessions health` prints a `went nowhere` line; as of 2026-08-27 it
  prints a dash, because no run in the store predates nothing — the sensor is
  newer than every row. **Every threshold in `boredom.rs` was argued rather
  than measured** — three identical outcomes, six, three notices a run — and
  the same is true of `step.rs`'s silence on the common path. What to look for
  once a corpus exists: a boredom rate near zero means the constants are too
  slow to be worth the mechanism, and one near one means the transcript is
  mostly the harness talking about the harness. Neither is visible any other
  way.

- **Rung 9's first two pieces shipped 2026-08-27/28** (§10, §10.1 —
  `appraisal::for_session` (`mecha-core/src/appraisal.rs`) is now the
  one assembly `mecha sessions appraise` and `mecha distill`'s episode
  tagging both call; `distill::upsert_args` (`mecha-core/src/distill.rs`)
  writes `meta.affect`/`meta.goal_errors` onto the pushed pkg episode, not
  gated on taint, and the quarantined `Distiller` now also extracts
  `Surprise { predicted, actual, about }` — gated like a correction, printed
  by `mecha distill` for a human to chase with `mecha gossip --entity
  <about>`, never auto-run. See HISTORY's 2026-08-27/28 entry for the
  four-round review saga on top of it). **Review-queue salience — the rest
  of §10 — is still unbuilt**: it needs the private
  `personalized_knowledge_graph` repository (a different codebase mecha only
  reaches through the MCP tool surface) to read `meta.affect`/
  `meta.goal_errors` back and reorder pkg's review queue on them. Not
  started, and not scoped beyond `GOAL-SYSTEM-DESIGN.md` §10's own paragraph
  naming it. Rung 8 (goal-closure appraisal) and rung 10 (the charter) are
  open PRs from other sessions as of this writing — #99 and #100 — and their
  content is theirs to describe, not this entry's.

**Two things named rather than done**, recorded so they are not rediscovered
as oversights. (The third — context pressure absent from `Homeostat` — shipped:
`peak_prompt_tokens` and `peak_context_pressure` on `Homeostat`.)

- **`/queues` still walks the stores itself.** `backlog.rs` is the shared walk
  and the homeostat uses it, but rewiring `mecha review`'s `collect_queues`
  (`commands/review.rs`) onto it needs either a wider `Backlog` or the loss
  of its per-item detail lines (*"3 drafted with the trifecta armed"*). That is
  a design choice, not a mechanical port. Doctor is deliberately further out —
  its per-store error isolation is load-bearing.
- **`/slots` is not sampled.** It is the best load signal available —
  occupancy directly rather than by proxy, and a second witness to prompt-cache
  reuse that `cache_lens` cannot get — but nothing reads it yet, and a sensor
  with no consumer should not put an HTTP call in the path of every run's
  start. `homeostat.rs` says so at the point where it would go.

**A session resumed from disk starts unanchored.** The pressure series now
survives a run boundary, so chat and the TUI predict from the first turn — but
a transcript records what runs cost *in total* and never what the last request
weighed, so a resumed session predicts from its second turn on. Closing it
means recording a last-prompt-size beside `peak_prompt_tokens`; cheap, and
nothing depends on it yet. Verified still open 2026-08-27: no `last_prompt_*`
field exists.

**And a step whose span crosses that same boundary gets no reading at all.**
`step.rs` differences the run's own trace, which restarts with the run, so a
mark taken in an earlier run is *unmeasurable* rather than empty — deliberately,
since the arithmetic alone would saturate to zero and announce the null step on
the commonest shape chat has. It is the same missing anchor as the item above,
one mechanism over, and a conversation-scoped work counter would close both.

**The compaction threshold is still a default nobody chose, and only half of
it is fixable by machine.** `AgentConfig::compact_at` derives two thirds of
the window, so the live providers sit at 173,015 (262,144) and 21,626
(32,768). Measured at `max_tokens = 8192`, the worst un-priced tail is 8,192
reply tokens plus the output budget — 16,192 at 262k against 89,129 of margin
(5.5× oversized), and 12,288 at 32k against 11,142 (**negative by ~1,100**).
Predictive compaction fixes the 32k side, because that overflow happens
between the check and the next request whoever decided. It may not fix the
other: §7.3 forbids relief compacting late, so no disposition may argue for
holding more context, and lowering the 262k threshold stays a person setting a
number. Left unset deliberately — but a reader should know it is a default
rather than a decision, and that the arithmetic above says the constant is
wrong in *both* directions.

**Not to be re-litigated** (all argued in the design): decomposition runs
downward and appraisal upward; the affect label is derived, never
self-reported; affect may only *narrow*, so a disposition is a monotone layer
above a structural check and never a replacement for one; and affect is a
priority function, never an objective.

### Validation — the probes could never run, and now they can

**`mecha validate`'s steer and denial probes had never run on an interactive
session, and `validations.jsonl` has been empty since it shipped while the
nightly reported success every night.** Fixed 2026-08-27 across two merged
PRs: `surface.rs` and the registry fix are **#90** (`c630ff94`); the
probe-specific half — driving the replay and filling `controllable` from
whether it arrives at the intervention anyway — is **#91** (`f0ca8ca`), which
is also where the `Regret` label below (`Agency::Own` + `controllable:
Some(true)`, `mecha-core/src/appraisal.rs:692`) actually gets produced. A skip
counted faithfully, and nothing read it as *this entire class is
unreachable* — the same shape as a rate over a zero denominator printed as
zero, which this file names in four other places.

`replay_registry` refuses to build when a recorded tool is missing from the live
registry, which is right: the tool list is the front of the cached prefix, so a
smaller toolbox is a different agent. What nobody had noticed is that three
tools are registered **only by a front-end** and so no CLI process has ever held
one — `ask_user` (246 of 408 sessions), `recall` (122), `show_file` (55). The
coupling closes on itself: a probe needs an intervention, interventions happen
interactively, interactive sessions carry `ask_user`. **319 of 407 sessions, and
it is the 78% that contains every intervention.** A `surface_only` registry
supplies them, consulted only where nothing executes; the store goes from **22%
replayable to 76%**.

The residual 97 name a `pkg__*` or a `google__*` and stay refused, which is the
bail working. `google__*` was renamed to `mail__*`, so **those 23 recordings are
permanently unreplayable**.

**With the probes finally running, 12 of 13 come back inconclusive**, and the
cause is found. On a pinned seed and a verified-quiet box the result is
byte-identical across two runs — 11 of 13 exactly, 2 differing by ±1 — with
divergence indices `#0:2 #1:6 #2:2 #3:1 #5:1`, **median 1**, against steer
points at 3 through 33. One session contributes six probes with steer points
from 10 to 33 and **all six diverge at the same call**: a trajectory-dependent
cause cannot do that, and a per-session constant can.

That constant is the tool surface. `RunConfig` kept the system prompt in full —
*"the text lets a replay rebuild the request"* — and kept tools as **names**,
with a comment naming three risks: added, removed, renamed. Those almost never
happen; **re-describe happens constantly**, and a name list cannot see it. Tools
render *before* the system prompt, so the replay rebuilt the back half of the
prefix byte-exactly and the front half from today's registry.

`surface.rs` records the specs once per distinct surface, content-addressed,
cited by `RunConfig.tools_hash` — costed rather than preferred, since 69 KB of
specs against a 25 KB average session would have quadrupled the store. Three
states, and `Unknown` is the load-bearing one: a recording from before the field
can never read as *matching*, and `Differs` needs no blob at all, so legibility
lands the day it ships and rebuildability accumulates after.

**Open:**

- **Nothing rebuilds a surface *from* a blob yet.** That is the rebuildability
  half; it is worthless until blobs exist and should be designed against a real
  `Differs` case rather than a hypothetical one.
- **The fix recovers nothing already recorded, and nothing can.** Those
  recordings never held the specs. **The appraisal corpus and the validation
  ledger start from zero the day this ships** — read that before budgeting on
  the 120 sessions on disk.
- **The prediction is untested and cannot be tested on the current corpus.**
  If the surface is the cause, restoring it should push the divergence index up
  and yield should follow the fraction of steers inside the new window. The 13
  existing probes have no hash and stay `Unknown` forever, so the earliest
  honest measurement needs sessions recorded *after* this lands. Re-running the
  13 and reading anything into the number would be false, and the temptation is
  real.
- **Whether `inconclusive` should stay one counter.** It now splits by fidelity
  (`matches` / `differs` / `unknown`), which is what keeps *drift* apart from
  *no evidence*.

**Attribution.** The reach and yield numbers, the determinism runs, the
divergence distribution and the fidelity split are the probe lane's
(`mecha-80`, 2026-08-27) and are **not independently reproduced here**. The
78%/22%→76% store measurement, the 69 KB-vs-25 KB costing, `surface.rs` and the
`tools_hash` field are this lane's. The one number both lanes measured
separately is the pinned seed — 387 and 388 of ~394 sessions at `seed: 42`, a
boundary apart and not a disagreement.

**One result to note.** With the probes running, the first non-neutral label
this system has ever produced appeared: `labels: {neutral: 119, regret: 1}` —
one session where the replay went elsewhere without the steer, carried the whole
way from probe through `apply_probe` to `affect_of`. One of 120 is not a result;
it is the seam working end to end.

### Cheap, and worth doing first

- **Decide whether replayed reasoning stays unbounded.** As of 0.1.2 the
  OpenAI-compatible backend sends every `Block::Thinking` back, which is what
  the model's own template expects and what took the empty turns from 6/6 to
  0/6. The cost is bounded in *compute* — the prefix cache absorbs it, measured
  at better than 95% reuse — but not in *context*: on the 08-10 run the model
  averaged ~930 output tokens a turn, most of it reasoning, so an 80-turn trial
  carries roughly 75k extra tokens by the end. Against a 173,015 threshold that
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
  its own.** `AgentConfig::compact_at` derives `COMPACT_FRACTION` (0.66) of the
  window when `[agent] compact_at_tokens` is unset, so raising `-c` from 32768
  to 262144 took the threshold from 21,626 to **173,015** as a side effect
  nobody chose. Nothing is broken by it — prompt caching means a growing
  transcript is only prefilled at the delta — but a cache *miss* at that depth
  costs ~120s of prefill before the first token, and a model's useful context
  is generally shorter than its trained one, so a transcript allowed to reach
  173k may be answered worse than one compacted at 100k. **The arithmetic that
  should decide it is in the goal-system section above**, which measures the
  margin at both live window sizes and finds the constant wrong in *both*
  directions — and explains why only one of those is a disposition's to fix.
  Stated once there rather than twice, because two statements of an open
  question drift apart.

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

The arc is complete, running nightly, and — as of 2026-08-23 — **producing
rules**. The provenance starvation measured on 2026-08-22 was resolved by
moving the evidence to the trusted side of the ruling rather than loosening
it: `learning::evidence_for` (`mecha-core/src/learning.rs`) hands the
reflector only user-typed words plus registry-owned tool names when coverage
cannot prove clean, `Evidence::UserTurns` records what was shown, and the
fail-closed gate is untouched — there is still no knob that lets a
full-context reflection out of an untrusted conversation. The
`--remine-untrusted` backfill recovered 11 lessons from the archive and the
store's first-ever proposal (5 rules) is staged; doctor's starved-learner
finding has cleared. The arc is in HISTORY under 2026-08-23.

What is missing beyond that is refinement:

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
  stage but cannot ask. (2026-08-24 sharpened the premise without changing
  the conclusion: `Asker::ask_in` proves a *shared* agent can route a
  question to the right present human — the web surface does it — but
  unattended still means nobody to route to, which is what this item is
  about.) The elicitation that grows autonomy is *policy*
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

### Mail as a surface you work — built; what is open is judgement

**`docs/MAIL-UX-DESIGN.md` is the authority** and its **§7** is where the
remaining questions live. `docs/MAIL-CORPUS-RESEARCH.md` is the measurement
that reordered the plan — **gitignored**, on `OPERATIONS.md`'s split: the
lesson is here, the figures are not.

**Every phase the plan named is shipped.** Phases 1–3 landed 2026-08-18;
**phases 4′ through 6 landed on 2026-08-19 between 11:08 and 18:10**, in
commits that name them by number (`3547cb1` … `a34a1c2`). The arc is in
[`HISTORY.md`](HISTORY.md). Verified against source 2026-08-25:

| Phase | What it is | Evidence |
|---|---|---|
| 4′ pre-filter | Bulk + automated-sender rules ahead of the classifier, only ever producing `ignore`, reading the envelope and never the body | `mail_triage.rs:340` (`prefilter`), `:303` (`PrefilterRule`) |
| Corpus eval | The classifier graded offline against mail whose outcome is known | `commands/mail.rs:333` (`Eval`), `:1307`; `--sample/--seed/--prefilter-only/--out` |
| 4″ tasks + `needs-info` | A thread becomes a board task, or parks naming what is missing | `commands/mail.rs:1937` (`task`), `:2020` (`needs_info`) |
| 4‴ day two | The aged set as a store-side primitive, surfaced by the briefing | `commands/mail.rs:504` (`list`, `--aged --aged-hours --surface`); wired live in `~/.mecha/triggers/morning.toml:59` |
| 5 `/mail` | The queue as a modal, with forward finally bound | `tui/mail.rs:401` `t`, `:403` `n`, `:405` `r`, `:406` `f` |
| 6 corrections | Field-level corrections feeding the few-shot pool *and* `triage`-domain reflections | `mail_triage.rs:600` (`Correcting`), `:626` (`apply_correction`), `commands/mail.rs:1804` (`reflect`) |

`mecha mail score` (`commands/mail.rs:488`) grades the *live* store, which is a
different question from `eval`'s corpus and is deliberately a separate verb.

**Two decisions recorded rather than left open**: tags are mecha's own and never
provider labels, and no mail parser belongs in mecha — the graph already ingests
`email.thread` episodes (`sources/mbox.rs`) with the bulk filter and the
`NEVER_AUTO` guard, so the live path pushes evidence through `kg_upsert` on the
`distill.rs` pattern and lets the graph extract. Push only `respond`/`notify`
buckets. **Front-door routing was the original phase 4 and is deleted**
(2026-08-19), along with `ROUTABLE_TYPES`, `is_routable` and
`Proposed::Frontdoor`. Do not rebuild it: `MAIL-UX-DESIGN.md` §1 has the five
reasons.

#### What is actually open

None of it is a phase. All of it is judgement, and **§7 of the design doc is
the authority** — restated here only far enough to be choosable:

- **The day-two age.** `--aged-hours` defaults to 30. The reply cliff falls at
  24 hours, but a briefing firing the morning after an evening email nags about
  something a working day old. §7.3 leans to one working day, and this is
  measurable against the corpus rather than pickable.
- **How `student-advising` is answered.** The largest category by a wide
  margin, and probably a *substitution* problem rather than an automation one —
  the same handful of questions (prerequisites, petitions, transfer credit),
  which is the profile of something a form or a published answer removes. §7.5,
  and the biggest single piece of the load.
- **Whether `meeting` earns a request kind.** Real volume and the *highest*
  reply rate of any category, which cuts both ways; structurally the greediest
  label, and the booking flow may already cover it. §7.4.
- **Retention on `~/.mecha/mail-triage/`.** Nothing prunes it. `mecha work
  clean` has the policy shape, but an archived verdict is also the eval fixture
  and the few-shot pool, so deleting costs what sweeping the work directory
  does not. §7.6.
- **Whether `t` can point back at the thread.** `kg_task_create` has no field
  for it, so it lives in the name or needs a pkg change. §7.2.

**Local state in no repository**, and the next session will want it:

- The classify timer is installed at `~/.config/systemd/user/`.
- `~/.mecha/mail-triage/` holds **264 records — 222 dartmouth, 42 personal**
  (2026-08-25; personal was 0 until that date), classified across several
  binary generations, so `request_type` is not consistent across the store and
  the taxonomy changed again on 2026-08-19. It wants one `mecha mail classify
  --limit 50 --force` sweep before any measurement is taken from it — note the
  flag no longer names an account, and `--limit` is **per account**, so the
  sweep is twice the size it used to be. Five of those records carried
  `proposed: frontdoor` and now read as `none` — by design, not by damage.
- **The corpus is at `~/.mecha/mail-corpus/{dartmouth,personal}.jsonl`**,
  owner-only, outside the repo, `*.jsonl` gitignored so it cannot be staged.
  both accounts complete as of 2026-08-19. Do not re-fetch to re-run an
  analysis; do re-fetch if the window needs extending. The personal half was
  fetched twice — the first was truncated by a 500-per-month cap, now fixed and
  reported when reached.
- **The personal account is in the nightly as of 2026-08-25**, reversing the
  entry that stood here. The measurement behind the old conclusion was right —
  personal mail is far more machine-generated and carries almost no
  correspondence needing an answer — and the conclusion drawn from it was
  backwards. Machine-generated is exactly what the prefilter disposes of for
  free: the first both-account sweep read 100 threads, and **47 of the 51
  candidates were disposed without a model**, 4 reaching the classifier. The
  account excluded for being expensive is the cheap one. What remains true is
  the *reason* it was excluded originally — the Google grant's 7-day expiry —
  and that is now survivable rather than fatal; see the classify entry under
  Traps.

### TUI polish

- **Steering and queuing are the same key.** Enter starts a run when idle and
  steers one already going; there is no way to queue a follow-up instead.
- **No `/export` or copy.** `NAMES` lists twenty-five commands
  (`tui/command.rs:314`, re-counted 2026-08-25 after `/entity` landed;
  twenty-four on 2026-08-24, twenty-one on 2026-08-21,
  after `/docs`, `/send` and `/remote-control`) and none of them get the
  transcript out. **OSC 52 is no longer the answer to assume.** `/docs` writes
  the link to the system clipboard that way (`tui/docs.rs`,
  `clipboard_escape`), and this file used to record it as the mechanism an
  export would use because the escape rides back down the SSH connection — but
  the 0.1.11 arc found no terminal acknowledges the write, which is why that
  release had to hand the mouse back and add `^s` selection mode instead. The
  working route off the screen today is a human selecting text, so an export
  wants a file, not a clipboard escape.
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

- **Notes have a home and no way in.** The graph already treats notes as a
  first-class *user-authored* source — `reflect.note` is an episode kind keyed
  by the note's stable id, `content_hash` catches an edited note as an update
  rather than a duplicate, and `reflect-process` promotes a structured note
  (`Type: #person/#company/#book`) into entities with identifiers and facts.
  That is the hard half and it is built. What is missing is **ingestion**:
  Reflect appears nowhere in `INTEGRATIONS.md`'s sources table, so notes reach
  the graph only if something else puts them there, and no other note
  application is understood at all.

  Worth doing because notes are the highest-confidence source the graph has —
  they are the user's own words about their own world, where mail and Slack are
  other people's — and because the mecha side already advertises the graph as
  holding "who people are and what you already promised", which is exactly what
  people put in notes.

  **The machinery already exists in both shapes this needs.** `imessage` reads
  a local SQLite file with a path argument, and `slack` authenticates with a
  user token — so a notes importer is a question of *which application*, not of
  new source infrastructure. The two families to price:

  - **Local-file apps** (Obsidian, Logseq — markdown vaults; Apple Notes and
    Bear — local SQLite) follow the `imessage` pattern: a path, a reader, and
    on Apple's stores the same Full Disk Access caveat that one already
    carries. No credential, no network, no rate limit.
  - **API apps** (Notion, Reflect) follow the `slack` pattern: a token, and
    whatever the vendor's export granularity turns out to be.

  The first step is a survey of which of these actually expose a stable read
  path today rather than a guess at it — the `mecha-graph source add <kind>`
  surface is the same either way, so the work is deciding what to support, not
  how. **Do not let this be advertised before it is built**: the
  mecha introduction's integration table deliberately omits notes for exactly
  that reason, and should gain a row only when a source exists.

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

  **`mecha review` (2026-08-22) runs against that rule on purpose, and the
  next person should not read it as an oversight.** By the sorting rule its
  graph half belongs in mecha-graph. It is in mecha-cli because the thing
  asked for was a *unified* surface — one list holding the graph's queue
  beside the outbox, the front door and the staged rule changes, none of
  which mecha-graph knows about. It touches no schema and holds no store; it
  drives `mecha-graph` as a child process, so the coupling is a binary name
  and a flag set rather than a crate. If the inversion happens, `review`'s
  graph verbs are the part that stays, and what moves is whatever
  `mecha-graph review` grows to make them thinner.
- **The queue is a ratchet, and now the instruments say so.** 91.2% of the
  6,434 pending candidates can never reach the accept gate: it wants a
  `(proposer, predicate)` on `DURABLE_CLASSES` (`precheck.rs:837`) or an
  earned rung, and `ladder.rs:120` has `Staged → Sampled → Trusted` with **no
  rung below Staged** — a class earns its way into autonomy and can never earn
  its way out of the queue. The symmetric fix is a `Rung::Suppressed` entered
  on the Wilson *upper* bound, mirroring the promotion rule and reusing
  `wilson_lower_bound`'s own arithmetic.

  **Do not ship it yet, and the reason is the finding.** On human verdicts
  alone, with a floor of ten, that rule currently suppresses **nothing** —
  there is no class the owner has judged often enough and rejected
  consistently enough to condemn. 40.5% of the queue sits in 660 classes with
  no human verdict at all. The prerequisite is evidence, which is what
  `review --sample` was built for: twelve random items across the ten largest
  unjudged classes is 120 decisions covering 1,597 queued items. Suppression
  is the last step, not the first.

- **The ladder reconcile shipped its promote half; the demote half is a
  decision still open.** `mecha-graph ladder` (2026-08-23) prints every
  class's rung, human record, Wilson LB and pending count, and
  `ladder --promote` re-derives rungs one rung per pass — run live, it
  unstuck `works_on`/`member_of`/`located_in` and the queue fell 7,296 →
  6,569 the same day. Deliberately **never demotes**: demotion stays
  correction-driven (D3), so `llm/works_at` still sits at `sampled` on an LB
  of ~0.49 — under the 0.65 floor, auto-accepting nine in ten on evidence
  that no longer justifies it. What remains is that ruling (statistical
  demotion, or leave D3 as the only path down) and wiring `--promote` into
  the nightly before precheck so threshold changes take effect without a
  review session. Note `reviewed_by` (V017, same day) now labels machine
  accepts, so the human record the recompute reads is exact going forward.

- **The nightly alert reports depth, not rate.** `ALERTS: merge queue 6491`
  reads identically whether the accept lane admits 5% of inflow or 95%, which
  is why the queue grew 1,035 → 6,434 over nineteen days unremarked. Print the
  delta and the admit rate (`admitted 56 of 971 added`), and have `doctor`
  raise a finding when admitted trails inflow three nights running. Related:
  vet's ceiling is `VET_LIMIT` × 10 predicates = 400 a night against 500–970
  candidates arriving, so the drain is specified slower than the fill.

- **A queue verdict has no undo, and the cascade raised the stakes.** One
  keystroke now decides up to a couple hundred items and nothing can walk it
  back: `mecha-graph undo` covers only episode delete/edit, a reject keeps
  its row (`status='rejected'`) but no verb flips it, and an accept asserted
  a live fact. The `reviewed_by = 'cascade:<seed>'` label makes the natural
  undo unit cheap to find: an `undecide <seed>` that returns the seed and
  its labeled fan-out to `proposed` (retracting accept-created facts only
  while nothing has built on them), a `mecha review` passthrough, and `u` in
  the `/queues` modal for the sitting's last verdict. Designed in
  conversation 2026-08-23, not built — and now covering a second caller:
  the phone's sample deck (2026-08-24) verdicts through the same CLI verbs,
  one thumb-tap each.

- **Slack verdict cards are deliberately unbuilt.** The `queues` command
  word is read-only; mail-triage cards and queue-verdict buttons each need a
  design pass against `SLACK-ACTIONS-DESIGN.md` first (closed Action enum,
  confirmation ladder — a group cascade from a phone is one tap with a
  two-hundred-item blast radius). Write the design before the code.

- **Extraction progress is unobservable, and the number standing in for it
  measures something else.** `HealthStats` has no backlog field, so
  `~/.mecha-graph/nightly.env` justifies `EXTRACT_LIMIT=400` by citing
  `enriched_pct` — which counts rows in `episode_enrichment`, written from
  exactly one place, `sources/bee.rs`, with `model = "bee-native"`. It equals
  2,411/20,501 = 11.7604%, the Bee share of the corpus, to the digit, and
  moves only when Bee ingests. The real backlog is episodes absent from
  `extract_state` at the current `prompt_version` (`extract.rs:229`) — roughly
  17,600, about 44 nights — and bumping `PROMPT_VERSION` re-queues the whole
  corpus. Add the field; fix the comment before it sizes another batch.

- ~~`mecha-graph fork` is broken~~ **Fixed 2026-08-26 evening** (graph
  `237b686`), so there is a working test bed again — `fork --out …` completes
  on the live 202 MB store in 1m38s with counts matching exactly. It was
  never a bug in `fork`: all three copy paths run migrate-then-copy, the
  harrier switch (2026-08-20) left the source's `vec0` tables wider than
  `run_migrations` builds them, and the reconciliation had been added to
  **one** of the three. `encrypt_in_place` was broken too and nobody noticed,
  because `fork` is the only one of the three people run on a whim — so a bug
  in two paths read as "forking is broken". Now in `copy_all_tables`, which
  every path calls. **A step every copy path needs belongs in the function
  every copy path calls**, and this trio has now broken together twice from a
  change to the destination's schema made before the copy; the first was a
  migration seeding a node.

- **Gossip's rotation cannot fill its quota.** `probe-targets` returns exactly
  10 candidates; `GOSSIP_ENTITIES=3` with `GOSSIP_COOLDOWN_DAYS=7` demands 21
  distinct targets a week from those 10, so nights silently under-fill (2 on
  08-22, 1 on 08-17, 2 on 08-16). Either drop to 2 a night, cut the cooldown
  to 3 days, or do the upstream fix `nightly-mecha.sh` already names — stop
  counting gossip's own reads as `retrieval_touch` demand. The
  self-reinforcement it warns about is visible in the current ranking: Frank
  Chang leads at 26 touches *because* he was probed.

- **`--create-subjects` will mint the next placeholder, and nothing stops
  it.** The 2026-08-26 repair merged away 30 topic nodes whose *display name*
  was another node's id, all created by `accept --create-subjects` answering
  `cannot resolve subject` on a candidate whose subject was never a name. The
  producer that fed it is fixed; the verb is not. It takes whatever string is
  in `subject` and makes a node named that, so the next producer writing
  anything unresolvable — `kg_upsert` shares the payload shape, so an agent
  can — repeats it exactly, and the damage is invisible afterwards because the
  placeholder makes the bad value *resolve*. The cheap guard is to refuse an
  id-shaped subject outright (`linkers::looks_like_node_id` is already the
  predicate, already tested); the fuller question is whether a verb that
  creates an entity as a side effect of a verdict belongs on a review surface
  at all. Rerun `mecha-graph repair-id-payloads --dry-run` to check; it is
  idempotent and reports zero when clean.

- **The graph nightly runs a binary no install refreshes — a seventh surface
  the update skill does not cover.** `scripts/nightly.sh:46` sets
  `PKG="$REPO_DIR/target/release/mecha-graph"` and *executes* it; it does not
  build. So a `cargo install` of both graph crates leaves the 01:30 run on
  whatever `cargo build --release` last produced in the repo tree, which on
  2026-08-26 was three days old — old enough that the night after the
  linker fix would have re-staged up to `KNN_MAX_CANDIDATES` (40) of exactly
  the candidates that day's repair had just cleaned out of the store. Caught
  before it fired. **Half-closed the same evening**: the `update` skill now
  names it as a seventh binary with its own `cargo build --release` and a
  date check, and warns against assuming `cargo install --path` refreshed
  that path (`80750bb`; the skill's own text had claimed the nightly
  "builds"). The binary itself was rebuilt, so the 01:30 run is safe.

  **What is still open is the better fix**: have the nightly build what it
  is about to run, so it cannot go stale between deploys at all. A skill
  step depends on somebody following a skill; a script that builds its own
  binary depends on nothing. The general shape is that "install" and "every
  binary that runs" are different sets — the same confusion the skill
  documents one level up, and an inventory of "what is running" that only
  lists things with a `--version` will always miss a cron job.

- **A stranger-facing README pass.** The public README still reads like the
  private repo's; nothing in it walks a person from `cargo install
  mecha-graph` to a populated graph.
- **Cosmetic**: the private checkout still lives at
  `~/Github/personalized_knowledge_graph` (paths baked into mecha's config
  `command =`, two crontab lines, and the gitignored OPERATIONS.md), and
  mecha's CLAUDE.md still says "pkg" in narrative spots.

### Larger, and deliberately not started

- **Canvas — researched 2026-08-21, blocked on a credential, and the
  unblocking action is a five-minute form.** `docs/CANVAS-RESEARCH.md` is the
  authority. The shape is settled: a **fifth binary on `mecha-mail`** (not
  core, not a fifth crate, and *not* a third-party MCP server — the capability
  override in `mecha-core/src/config.rs:804` is per **server**, so forcing
  `external_send` to cover a grading tool makes every read a send sink and the
  interlock blocks reads the moment a submission arms taint), OAuth2 over
  `urn:ietf:wg:oauth:2.0:oob`, ~16 tools, credentials at
  `~/.mecha/canvas/<account>/oauth.json` on its own root.

  **Nothing can start until Dartmouth issues a credential.** Self-service
  tokens were decommissioned 2025-06-18 and the probe on 2026-08-21 confirmed
  the `+ New Access Token` button is greyed out for faculty, not just students —
  so the service request form is the only door, at roughly five business days.
  Ask for a scoped developer key *and* name a manual token as fallback in one
  submission, and ask for **"Allow Include Parameters"** on the key or scoped
  tokens silently ignore `include[]` and return 200 with the data missing.

  The design decision that makes the rest affordable: **assistive, never
  evaluative**, with completion checking as the sole exception. No verb accepts
  a numeric or letter grade. Two things to verify against the real key before
  relying on them, both recorded as §8 questions: that a scoped key genuinely
  cannot reach GraphQL (which is what makes posting grades structurally
  impossible, and rests on undocumented Instructure behaviour), and whether a
  manual post policy hides submission *comments* or only grades.


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

- **Images: built, merged and deployed 2026-08-21.**
  `Block::Image`, both provider encoders, `image.rs`'s caps, `provider::preflight`,
  the `scripts/mmproj.sh` guard and four entry points (Slack connector,
  remote-control inbox, `mecha run --image`, and a file dropped on the TUI
  prompt) are on `main`, 1,210 tests green. Deployed the same day, in the
  `update` skill's order:

  - **`:8080` serves vision.** `llama-local.service`'s ExecStart is the
    development checkout, so the merge is what delivered `--mmproj`; the
    startup line now reads `loaded multimodal model, …/mmproj-BF16.gguf` and
    `/props` reports `modalities.vision: true` with `n_ctx_slot = 262144`
    unchanged. Measured after the restart rather than assumed: **72–79 tok/s**
    on a short prompt against the 70.5 the script records for `-np 4`, so the
    projector costs nothing in generation and the server did not load under
    memory pressure — the failure that would have left it permanently slow.
  - **The binary is reinstalled** and `mecha-slack`, `mecha-triggers` and
    `mecha-drain` restarted onto it, each verified from its own log line
    rather than from `is-active`. Worth keeping from this one: **the version
    string did not change**, because the arc bumped no version — so
    `mecha --version` proved nothing, and what proved it was `mecha run
    --help` carrying `--image` plus cargo reporting that it had replaced an
    install made from the *remote-control worktree*.
  - **Verified live** on the production config against the screenshot that
    started this: quoted back verbatim, and with no preflight warning, because
    config and server now agree.

  One trap found on the first live retest, and now in the `update` skill's
  ordering constraints: **a TUI started before the install kept running the
  old inode** (`/proc/<pid>/exe` → `mecha (deleted)`), and because the config
  had meanwhile gained `vision`, its *call-time* config re-read in `show_file`
  failed with a parse error two hours after the change. `deny_unknown_fields`
  is right and is why — the cost is that config is a wire format between
  versions, and long-lived processes must be restarted after a config key is
  added, not only after an install.

  Worth a look if it recurs: `show_file` loads the **whole global config** at
  call time for one number (`slack.max_upload_mb`), which is what couples an
  unrelated section's strictness to a tool call mid-run. Capturing it at
  registration would decouple them.

  **`~/.mecha/config.toml` gained `vision = true` on `[providers.local]`**, and
  that file is in no repository — a fresh clone gets the code, the projector
  guard and the preflight, and still sends no image until somebody writes that
  line. Backup at `config.toml.pre-vision`. The projectors for qwen3.6,
  qwen3.8 and gemma-4-E4B were downloaded; gemma-4-26B's always was, unused.

  **Drag-and-drop onto the TUI prompt works locally** and is the third door.
  It is structurally impossible over SSH — a terminal pastes the *laptop's*
  path and the TUI resolves it on the far box — so testing it means a local
  terminal, which is what made it look absent.

  Also deliberately not built, and named so it is not rediscovered: **an image
  cannot join a run already in flight.** The steering queue is
  `VecDeque<String>` and a mid-run attachment still lands on disk and still
  has its path named in the steered text, so nothing is lost — but the pixels
  wait for the next turn. Widening the queue is a `RunContext` change.

- **Slack as a remote control — built and verified live; two things left.**
  The arc merged on 2026-08-09 (**PR #25**) and is described in
  [`HISTORY.md`](HISTORY.md) under 2026-08-09; the design authority is
  [`SLACK-DESIGN.md`](SLACK-DESIGN.md) and the evidence
  [`SLACK-RESEARCH.md`](SLACK-RESEARCH.md). What is genuinely unbuilt:

  - **`ask_user` is absent — and the structural half of that claim died on
    2026-08-24.** `Asker::ask_in(&ToolCtx, …)` (`mecha-core/src/tool/ask.rs`,
    default forwards to `ask`, TUI untouched) lets a shared `AskUserTool`
    route by the calling run's jail — the web surface ships exactly this
    (`serve/present.rs::WebAsker`, key = the workspace's directory name).
    What remains for Slack is now ordinary work, not architecture: an
    `Asker` mapping a thread's jail back to its thread, plus card rendering
    through the `Action` ladder.
  - **MCP tools do not honour the per-thread jail** — only the built-in tools
    do, because servers are spawned once with the agent. They are rooted at
    the `slack` producer directory so paths at least agree; isolation between
    threads is not there. **Re-scoped 2026-08-24: this is now every
    many-conversation front-end's limitation, not Slack's** — `mecha serve`
    inherits it identically (`serve/chat.rs`, servers rooted at the `web`
    producer). Closing it means an agent per conversation, an MCP startup
    each; the cost is why it stays open.
  - **The outbox review cards have not been exercised live.** Built and unit
    tested; no run has yet staged a draft while the connector was watching.
  - **It is installed and running** (`mecha-slack.service` active, re-verified
    2026-08-24 — on the current install at merge `9e20214`, bounced and
    inode-swept the same night).
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
