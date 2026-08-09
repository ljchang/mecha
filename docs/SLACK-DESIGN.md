# Slack as a remote control — design

**2026-08-09. Unbuilt.** How mecha is driven from Slack: one always-on
connector at home, a Slack thread as the unit of work, and the outbox as the
place a run's outbound actions wait for a human who is somewhere else.

The evidence, the alternatives, and the platform limits behind every decision
here are in [`SLACK-RESEARCH.md`](SLACK-RESEARCH.md). This document does not
restate them. It records what gets built, what each piece may not do, and what
is deliberately left out — that last part being the half a reader cannot
reconstruct later.

One sentence to carry through: **this is the TUI's front-end with a WebSocket
where the terminal was and an absent human where the present one was.** Where a
decision here differs from the TUI's, the absence of the human is why.

---

## 1. Shape

```
Slack  ──wss──▶  mecha slack connect        one always-on process at home
                   │
                   ├── thread store          ~/.mecha/slack/threads/<id>.json
                   ├── one Agent             one provider connection, one cached prefix
                   └── one task per thread   RunContext per thread: own jail, own budget
                                             own Conversation, own session
```

The connector is a **front-end**, in the same category as `tui` — not a server,
not a daemon that other things talk to, and not a tool. It owns the Slack
socket, maps `AgentEvent` to Slack, implements `Approver` and `Asker`, and
drives the CLI as a child process for everything that already has a CLI verb.

### Where the code lives

| Piece | Home | Why |
|---|---|---|
| Socket Mode client, Web API calls, file up/download, Block Kit types | **new crate `mecha-slack`** | It must not be able to learn about agents. A crate with no `mecha-core` dependency cannot, structurally |
| Front-end: event mapping, thread state, `SlackApprover`, `SlackAsker`, the run loop | `mecha-cli/src/slack/` | Exactly where `tui/` lives, for exactly the same reason |
| `mecha-core` | **untouched in phase 1** | The loop must never learn what a Slack thread is |

The fourth crate is worth its weight for one reason: it makes the invariant
checkable by reading `Cargo.toml` rather than by reviewing diffs. `mecha-slack`
depends on `reqwest`, `tokio-tungstenite`, `serde` — and not on `mecha-core`.
The alternative considered and rejected was putting the client in
`mecha-cli/src/slack/api.rs`: it works, it is less ceremony, and it makes the
separation a convention instead of a fact. `mecha-mail` is the precedent for the
split; this crate is smaller because it has no OAuth lifecycle and no MCP
binaries.

**New dependency:** `tokio-tungstenite` with `rustls-tls-webpki-roots`, matching
the existing TLS stack. `tokio`'s `net` feature has to be enabled; it currently
is not.

---

## 2. The thread state machine

Written first, because a remote-control API whose states have no documented
meaning cannot distinguish *waiting* from *wedged* — the failure named in
`SLACK-RESEARCH.md` §9. **Every state below has a meaning and a stated action
that resolves it.** A state that can only be left by restarting the connector is
a bug in this table, not in the code.

| State | Meaning | Resolved by |
|---|---|---|
| `unbound` | A thread exists in Slack; mecha has no session for it | The first owner message → `idle` |
| `quarantined` | The thread was started by a non-owner | A front-door record is written; the thread never gets a session. Resolved by `mecha frontdoor`, not here |
| `idle` | Bound to a session and a workspace; nothing running | An owner message → `running` |
| `running` | A run is in flight | The run ends → `staged` / `done` / `failed`; or the owner presses **Stop** → `cancelled` |
| `awaiting_input` | The run is blocked on an approval or an `ask_user` | The owner answers → back to `running`; or the timeout fires → back to `running` carrying a deny/decline (§5) |
| `cancelled` | Stopped at a safe point; the partial turn is kept | An owner message → `running` again on the same conversation |
| `staged` | The run finished and left drafts in the outbox scoped to it | Release or reject, from the thread's buttons or any other outbox surface → `done` |
| `done` | Finished, nothing pending | An owner message → `running` |
| `failed` | The run errored | An owner message → `running`; the error is posted, never only logged |
| `orphaned` | The connector restarted while this thread was `running` or `awaiting_input`; that run no longer exists | **Announced in the thread on reconnect**, then → `idle` |

`orphaned` is the state that exists because of the failure mode, and it is the
one most likely to be skipped. A run lives in the connector's process. If the
process dies, the run is gone, and a thread left displaying "working…" forever
is exactly the wedged-versus-waiting confusion. So on startup the connector
walks its thread store, finds every thread not in a terminal state, posts *"this
run did not survive a restart"* into each, and resets it. Silence is not an
acceptable way to report this.

**Thread state is a store, not memory**: `~/.mecha/slack/threads/<key>.json`,
one file per thread, written with the learning store's rules — one pretty JSON
per record, temp-sibling-and-rename, advisory flock for read-modify-write. Key
is `<channel_id>-<thread_ts>`. Fields: `state`, `session_id`, `workspace`,
`mode`, `last_seen_ts`, `run` (pid + started_at, present only while running),
`stream_ts`, `controls_ts`, `budget_spent`.

**A `run` marker whose pid is gone reads as *not* running**, with a range check
on the pid — the exact lesson the trigger store's `.running` marker records,
where `kill(-1, 0)` succeeding would otherwise report every dead run as alive.

---

## 3. Identity, and where the binding lives

Two tiers, per the research: an allowlist, and the front door for everyone else.
What this document adds is *where the allowlist lives*, which is a security
decision in its own right.

**The binding is not config.** `~/.mecha/slack/binding.json`, mode 0600, holding
`{team_id, enterprise_id, owners: [user_id], bound_at}`. Written only by
`mecha slack link`. The tokens sit beside it in `credentials.json`, same mode,
same directory — the pattern `~/.mecha/mail/` already uses.

The reason it is not in `mecha.toml` is the one triggers already established:
`[[hook]]`, `[[mcp]]` and `[[subagent]]` are declarable in a project file, which
is a file that arrives with a cloned repository. A repo that could declare a
trigger has been handed a cron slot on your machine — and a repo that could
declare a Slack owner has been handed **the remote control**. That is strictly
worse, so it gets the strictly stronger treatment: the binding is a store, and
the connector loads `Config::load_global()` like a trigger run does, global file
only, no project layer.

`[slack]` in the global config therefore holds tunables only, and nothing that
grants anything:

```toml
[slack]
enabled            = true
max_concurrent     = 3          # threads running at once
approval_timeout   = "10m"
ask_timeout        = "10m"
default_mode       = "ask"      # per-thread; changed by button, never by prompt
max_turns          = 40
max_cost_usd       = 2.0
max_upload_mb      = 25
stream_flush_chars = 800
stream_flush_ms    = 1000
```

**Two edits, not one.** Every field above must appear on `Config` *and* on
`ConfigLayer`, or the `[slack]` table becomes a parse error that kills startup
while every unit test stays green — the exact way hooks shipped unreachable.
`every_field_of_config_is_reachable_from_a_file` catches it; run it.

### Linking

1. `mecha slack link` on the workstation prints a nonce, valid ten minutes,
   once.
2. The user DMs it to the app.
3. The connector binds that `user.id` + `team_id` and writes `binding.json`.

The nonce proves shell access to the machine the agent runs on. Email does not:
`users:read.email` returns the workspace's claim about an address, and a
workspace admin can change it. `users.lookupByEmail` is used once, at link time,
for a friendly confirmation — never as the check.

**The check itself** — `payload.user.id ∈ owners && team_id == bound_team` — runs
on **every** inbound event, in the connector, before a message becomes a user
turn and before any button is honoured. It is not a tool, not a hook, and not
reachable by the model.

---

## 4. Runs

### Building the agent

Like the trigger runner: `GlobalOpts` constructed programmatically with
`global_config_only: true`, never parsed from argv. Unlike the trigger runner,
`interactive` cannot be `false`, because that is what withholds `ask_user` and
installs `ModeApprover`. So the connector follows the TUI:
`setup::prepare_with_approver(&opts, Arc::new(SlackApprover::new(tx)))`, then
inserts `AskUserTool::new(Arc::new(SlackAsker::new(tx)))` into the registry.

**Known touch point:** `setup::prepare_with_approver` swaps the approver *only*
when `permission_mode == Ask` (`mecha-cli/src/setup.rs:68`). A Slack run in
read-only mode would silently get `ModeApprover` instead. Either the connector
sets `Ask` explicitly, or that function learns to honour the approver it was
handed. Prefer the second — a function that ignores its argument under some
configurations is a trap for the next front-end too.

### The system prompt suffix

The trigger runner appends an `UNATTENDED` suffix. That is wrong here and would
be quietly wrong: a Slack run *can* reach a human, just not synchronously.
A distinct `SLACK` suffix says so — a human is reachable but may take hours,
`ask_user` is available and expensive, outbound actions are staged for review
rather than sent, and the answer will be read on a phone.

### Workspace and retention

**Producer `slack`, jail `~/.mecha/work/slack/<thread_key>/`.**

This is the one place where the obvious choice is wrong. A producer per thread
(`slack-<ts>`) looks natural and breaks retention: `work::clean` keeps the last
`[work] keep` **top-level entries per producer** and never removes a producer
directory, so a producer per thread means an unbounded pile of directories that
nothing sweeps. With one `slack` producer, each thread is one top-level entry, so
the existing policy retires old threads with no new code — and because entries
are counted rather than files, a thread's whole directory counts as one. Raise
`[work] keep` if ten threads of history is too few.

`work::ensure_outside_mecha_home` is satisfied by construction, as it is for
triggers.

### Concurrency

One `Agent`, one `tokio` task per active thread, each with its own
`RunContext` — the configuration `Agent::run_in` exists for. `max_concurrent`
bounds it; **at the cap the connector refuses and says so in the thread** rather
than queueing. A queued run that starts twenty minutes later, against a
workspace whose state has moved, is worse than an honest refusal.

**One process owns a session.** `session.rs` appends with no lock, so two
writers interleave records and each resumes from a transcript the other is
mutating. The connector owns every session it creates and refuses to attach to
one whose marker says another process holds it.

### Steering, cancelling, and the two things they cost

Steering is free and is the point: an owner message arriving while a run is
`running` goes into `RunContext::queued_input`, gets folded into the message
carrying the tool results, and the run keeps going.

Two consequences to accept out loud:

- **Every owner message in a thread is input.** There is no way to comment
  without steering — the same property the TUI has, where Enter both submits and
  steers. `HANDOFF.md` already lists that as TUI polish; if a queue-instead-of-
  steer affordance is ever built, both surfaces get it.
- **Slash commands do not work in threads.** Slack states it: commands "cannot
  be invoked in message threads." So in-thread control cannot be `/mecha stop`,
  and this is a platform limit rather than a preference. In-thread control is
  therefore **buttons**, which carry `payload.user.id` and are gated the same as
  everything else.

---

## 5. What the thread looks like

Three messages per run, and no more:

1. **The stream.** `chat.startStream` on the owner's message, appended to, ended
   with `chat.stopStream`. This is the answer and the progress, in one place.
2. **The controls.** One small message posted at run start carrying **Stop**,
   **Mode**, **Outbox**; `chat.update`d at the end into a terminal summary with
   the buttons removed, so nothing is re-clickable. It exists separately because
   Stop must be pressable *during* the stream.
3. **An approval card**, only when one is needed (§5.2).

Before the first token: `assistant.threads.setStatus`. Its two-minute auto-clear
is a feature — a connector that dies mid-run cannot leave a spinner forever —
and it writes only to the requesting user's own view, which is why it is not a
send sink.

### 5.1 Event mapping

`AgentEvent` → Slack, per `SLACK-RESEARCH.md` §5. Buffering is the only part
with numbers in it: flush a `markdown_text` chunk at **800 characters or 1000
ms, whichever comes first**, and never exceed roughly one append per second.
Size-based flushing is what Slack's own SDK does; the timer exists so a slow
model still shows progress.

Thinking deltas are dropped, not rendered. A denial is rendered as a
`task_update` **naming the layer that produced it** — interlock, hook, approver,
or timeout — because a policy refusal the human never sees reads as a mysterious
failure, and the thread is where the human already is.

`Done` closes the stream with a footer: turns, tokens, cost if priced, stop
cause, the session id, and the workspace path.

### 5.2 Approvals

**Approvals are the hot path, not the exception**, because the default mode is
`ask` (§5.3). That is a deliberate trade: the owner sees every state-changing
call and can widen per thread when a piece of work deserves it. It puts more
weight on the card being good and on the timeout being right than a read-only
default would.

Outbox routing is orthogonal and always on: staging happens before the approver
is ever consulted, so an outbound send is a draft regardless of mode.

When an approval is needed: a durable `card` — never an ephemeral, which does not
survive a reload and cannot be updated — carrying the tool, a summary, the
workspace, and Approve / Reject. On decision, `chat.update` it into
"approved by @x at T" so the record is permanent.

- **Gate on `payload.user.id`.** The button's `value` carries a correlation id
  and authorises nothing.
- **`trigger_id` lives three seconds and is single-use.** If Reject opens a
  modal for a reason, open it before doing any other work.
- **No `Answer::Always`.** It is process-local and never persisted, which is
  survivable in a TUI session that ends with the terminal and is a months-long
  blast radius in a connector.

**The timeout, and the string.** Silence must deny — that rule holds everywhere
in mecha and holds here. But the denial must not read `"Denied by the user:"`,
because that exact string is what the learning miner keys on, and an unanswered
2am prompt would otherwise become training data from a human who was not there.
It is the `"Blocked by a hook:"` distinction in a new costume: machine state read
as a human correction.

So the timeout returns:

```
No answer from Slack within 10m: the call was not approved because nobody answered.
```

and it ships with **a test asserting that string is not mined**, named on the
reason, exactly as the hook string has one. `ask_user`'s timeout returns `None`
(declined) using the existing decline wording **verbatim** — it was A/B measured
on this machine, and "proceed with your best interpretation" made the model
invent.

### 5.3 Mode, per thread

The controls message carries a **Mode** button opening a modal with `ask`
(default), `allow`, and `read-only`. Three rules, each with precedent:

- **Only an allowlisted owner may set it**, checked on `payload.user.id`.
- **It is set by a button and never inferred from prompt text**, exactly as the
  TUI's `/review` mode is — release and permission policy must not be decidable
  by anything sharing a context window with third-party text.
- **It is per thread.** Elevating one piece of work does not elevate the next.

**The mode lives on the thread's `RunContext`, not on the `Agent`.** This is the
part that is easy to get wrong: the TUI changes modes with
`Agent::set_approver`, which is correct for a front-end with one conversation and
would be a cross-thread leak here — one thread pressing *allow* would widen every
other thread sharing the connector's single `Agent`. Each thread's `RunContext`
carries its own `Arc<dyn Approver>`, and `SlackApprover` holds that thread's mode
behind a shared cell so a button pressed mid-run takes effect on the **next**
call rather than the next run.

Consequence worth stating: `SlackApprover` implements all three modes itself
rather than delegating to `ModeApprover` for two of them. Delegating would send
`read-only` and `allow` decisions down a path that never reaches Slack, so a
denial would be invisible in the thread — and it would re-expose the
`prepare_with_approver` trap in §4, which only swaps the approver when the
configured mode is `Ask`.

### 5.4 Reviewing drafts

The thread's **Outbox** button lists the drafts *this run* staged — scoped by an
id-diff between submit and completion, so no thread touches items another
session staged. Release and reject are `mecha outbox …` **child processes**,
spawned detached with a null stdin, and the result is collected by **polling the
store, not the process** — a child that died without writing is otherwise
indistinguishable from one still working. That whole pattern is lifted from
`tui/mod.rs` unchanged, including the caps that retire a wedged watch.

A tainted draft is shown in red with its full arguments and **never auto-
releases**. `/review auto` has no Slack equivalent in v1: an auto-release policy
set from a surface other people can post into is not a policy.

---

## 6. Files

### Out

Upload with **no `channel_id`** so the file stays private, then reference it from
an `image` block via `slack_file: {id}`. Upload and post with the same token or
the app cannot display its own file. Code and logs go in
`rich_text_preformatted` with a `language`; above `max_upload_mb`, or when the
artifact is a directory, the run publishes a bundle and the thread gets the URL —
which is the factory's half of the split, and which the outbox's existing
`publish` kind already models.

**`unfurl_links: false` and `unfurl_media: false` on everything the model
authors.** A model-emitted URL that unfurls is an outbound GET no tool call ever
made and the interlock never sees.

### In (phase 1 — no core change)

The file is downloaded into `<workspace>/inbox/<name>` and the prompt says where
it is. The agent then reads it with the tools it already has.

That last clause is load-bearing and is why phase 1 is not a compromise: because
the agent reaches the bytes through `fs_read`, which already declares
`private_data`, **the taint legs arm correctly with no new capability
plumbing**. A design that injected file content directly into the prompt would
have to arm taint by hand, which is the kind of parallel path that drifts.

Four download rules, all required:

1. Send `Authorization: Bearer <bot token>` explicitly.
2. **Disable redirect following** — `files.slack.com` redirects to
   `<team>.slack.com`, and HTTP clients strip `Authorization` across hosts.
3. **Reject `text/html` even at HTTP 200.** An unauthenticated fetch returns a
   sign-in page with a 200, not a 401.
4. Cross-check the byte count against the file object's `size`.

Without these, a Slack login page lands in the model's context labelled as the
user's screenshot. Also: no-op-ack `file_shared` even though `files[]` off the
message event is what gets consumed, or Slack retries three times per upload
against an unhandled subscription.

### The send-sink boundary

- **The reply address is set by the transport and there is no argument that
  changes it.** No `slack_post(channel, …)` tool exists. This is
  `Record::for_privileged_run`'s pattern applied to an address.
- **A DM reply is not a send sink** — the recipient is the principal.
- **A channel reply is.** A tainted run invoked from a channel answers in the
  owner's DM instead, and says in the channel that it did.

### What the connector will not read

**Only the owner's own messages become user turns.** The connector does not read
other people's messages in a channel thread into the conversation, ever. If
channel context is wanted later it arrives as a `slack_read` *tool* declaring
`untrusted_input` and marking its output `.from_outside()` — which is the only
shape that arms the interlock correctly. Reading third-party prose in through
the front-end would put untrusted text into the conversation with no capability
anywhere saying so.

---

## 7. The CLI surface

| Command | Does |
|---|---|
| `mecha slack link` | Print a one-time nonce; bind the first owner who sends it |
| `mecha slack connect` | The connector. What the systemd unit runs |
| `mecha slack status` | Binding, socket state, active threads and their states |
| `mecha slack threads [--state]` | List the thread store |
| `mecha slack catch-up` | Read `conversations.replies` since `last_seen_ts` for non-terminal threads |
| `mecha slack notify` | Read stdin, post to the owner's DM. What a trigger's `notify` calls |
| `mecha slack unlink` | Remove the binding; the tokens stay |

`notify` is the cheapest win in the whole design: a trigger's `notify` already
runs `sh -c` with the answer on stdin, so `notify = "mecha slack notify"` puts
the morning briefing on the phone with no new trigger concept at all.

**Operationally** this is a third always-on user unit beside `mecha-triggers`
and `mecha-drain`: `Restart=always`, `Environment=PATH=%h/.cargo/bin:…` (whose
omission has bitten this project four times), `ExecStart` from `~/.cargo/bin`
and never from a repo checkout, `loginctl enable-linger`.

---

## 8. Testing

| Layer | Covers |
|---|---|
| Unit, pure | The state machine's transitions; envelope decode and ack; `AgentEvent` → chunk mapping; the flush buffer's size/time rule; owner gating given a payload; the file-download guard rejecting `text/html` at 200 |
| Unit, scripted | A run driven by `ScriptedProvider` through the connector's event mapper, asserting the exact sequence of Slack calls against a recording client — the `ScriptedProvider` idea applied to the transport |
| Integration | A local WebSocket fixture serving canned envelopes, so reconnect and `orphaned` recovery are exercised without Slack. Skips when absent, and `MECHA_TEST_REQUIRE_BACKENDS=1` turns the skip into a failure |
| Live | Link, one run, one approval, one file each way. Once, by hand, recorded in `HISTORY.md` |

Two assertions that exist because of specific traps, and should be named after
them: the timeout denial string is **not** what the writing miner keys on, and
a thread's jail resolves under `~/.mecha/work/slack/` rather than the
reviewer's directory.

The structural blindness applies here as everywhere: a scripted transport
replays what we *believe* Slack does. The live pass is what covers the rest, and
the UNVERIFIED list in `SLACK-RESEARCH.md` §12 is its checklist — particularly
whether unacked envelopes replay, which decides whether `catch-up` is a nicety
or a requirement.

---

## 9. Build order

1. **`mecha-slack`**: Socket Mode connect, envelope ack, reconnect, `chat.*`
   including the stream trio, files both ways. No agent anywhere. Testable
   against the fixture.
2. **Binding and gating**: `link`, the store, the owner check, `status`. Nothing
   runs an agent yet — a bound app that echoes "I heard you, and you are the
   owner" is the security boundary working, and it is worth landing alone.
3. **The state machine and the thread store**, including `orphaned` recovery on
   restart. Before any run exists, so the states are designed rather than
   discovered.
4. **One run, end to end, *including* `SlackApprover`**: stream, controls, Stop,
   steering, approval cards, the timeout and its string. The approver is not a
   later step because `ask` is the default mode — a run without it cannot do
   anything but read. `SlackAsker` lands here too; it is the same shape.
5. **Mode buttons** (§5.3), with the per-thread `RunContext` wiring and a test
   that one thread's mode change does not move another's.
6. **Outbox review in-thread** — the piece that closes
   `PUBLIC-SURFACE-DESIGN.md` §11.
7. **Files**, out then in, with the four download rules.
8. **`notify`**, and the systemd unit.

Steps 1–3 are the ones worth doing carefully; 4–5 are the ones with the subtle
correctness conditions; 6–8 are wiring over seams that already exist.

---

## 10. Deliberately out of scope

- **`Block::Image` and true multimodality.** Phase 2. The enum variant is
  additive; the work is auditing every consumer of `Message::text()` that
  silently drops non-text blocks — compaction, the learning miners, the eval
  graders, `RunOutcome.text` — plus a live question about whether the local
  model accepts images at all. §6's workspace-path route delivers the capability
  without any of it.
- **A `slack_send` tool.** Posting to an arbitrary channel is a different
  feature: `external_send`, outbox-routed, and not needed to be a remote
  control.
- **Reading channel context into a run** (§6). Needs to be a tool, not a
  front-end behaviour.
- **`/review auto` in Slack** (§5.4).
- **A true "accept edits" tier** — file edits auto-approved while `shell` still
  asks. It does not exist in mecha: `PermissionMode` is `Allow | ReadOnly | Ask`
  and `ModeApprover::approve` ignores the tool input entirely, so `shell: ls` and
  `shell: rm -rf` are the same decision. Expressing it needs the per-command
  approval surface `HANDOFF.md` lists as a structural gap. Until then `ask` is
  the honest default and the Mode button is the escape hatch.
- **Feeding the front door from Slack.** A message from a non-owner is ignored —
  not quarantined, not recorded. The front door exists for people who were handed
  a form; a colleague in a channel has other ways to reach a human. Revisit only
  with real examples of Slack messages that should have become requests.
- **More than one owner tier.** Two tiers, or the front door.
- **Distribution, OAuth, multi-workspace.** An internal single-workspace app is
  what keeps the 2025 `conversations.history` throttle inapplicable; distributing
  it would forfeit that with no escape route, since a Socket Mode app cannot be
  Marketplace-listed.
- **Token rotation.** Opt-in, irreversible, 12-hour expiry, and it buys a
  single-user internal tool nothing.
- **Hosting any of this on the factory**, which must never be an owner channel.
  The box carries artifacts; Slack carries control.
- **Exposing mecha as an MCP server to Slackbot's MCP client.** The per-tool
  approval UX is genuinely good and it is still the wrong product: it needs the
  public endpoint Socket Mode was chosen to avoid, and it puts *Slack's* model
  in charge of deciding when to call mecha's `shell`.
- **Canvases, huddles, workflow steps, scheduled Slack-side automation.**
  `cron.rs` and `trigger.rs` already own scheduling; a second scheduler that
  lives in someone else's product is a second source of truth.

---

## 11. Decided, 2026-08-09

The three questions this document opened with, answered and recorded here so
they are not re-asked:

1. **A personal Slack workspace first**, with the institutional one possibly
   added later. The design does not change if it is — the binding already keys
   on `team_id`, so a second workspace is a second binding rather than a new
   concept. What *would* change is the "internal, undistributed" status that
   keeps the 2025 `conversations.history` throttle inapplicable; re-read §10
   before installing anywhere a policy could call that distribution.
2. **`ask` is the default mode**, with per-thread buttons to widen to `allow` or
   narrow to `read-only` (§5.3). Deliberately not "accept edits", which mecha
   cannot express today — see §10 for what it would cost.
3. **Non-owners are ignored.** No front-door record, no quarantine, no reply.
   The front door stays fed only by `mecha-factory-publish drain`.
