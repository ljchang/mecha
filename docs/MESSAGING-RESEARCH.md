# Inter-agent messaging — research

*2026-08-09. Research toward letting separate mecha sessions/processes message
each other — the way Claude Code's cross-session `SendMessage` lets one agent
coordinate with another. Three inputs: how Claude Code actually does it, what
the wider field has converged on, and what mecha already has. Ends with a
proposed shape. Nothing here is built.*

---

## 1. How Claude Code does it

Two separate features, both shipped 2025–2026:

**Cross-session messaging** (`SendMessage` / `ListAgents`):

- **Addressing** is by session *name* (user-set via `/rename`/`--name`, or
  auto-derived from the working directory). Discovery is registry files on
  disk; collisions disambiguated by showing each session's workdir.
- **Transport** on one machine is a per-session Unix domain socket
  (owner-only permissions are the security boundary; the socket path is
  exported to hooks/Bash as `CLAUDE_CODE_MESSAGING_SOCKET`). Cross-machine
  goes through Anthropic servers and is reply-only.
- **Delivery** is queue-until-safe-point: read *between tool calls* during an
  active turn — a running tool is never interrupted — and when the receiver
  is idle, a new turn is started with the message. Plain text only; never
  history, never files.
- **Admission** is receiver-side policy: `crossSessionInbound` =
  `accept` / `hold` / `refuse`, with the default derived from *both* parties'
  permission modes (a bypass-permissions sender messaging a prompting
  receiver is held for a human). Holding is distinct from refusing.
- **Loop control**: identical repeats dropped within a window, ~50 unread
  cap per session, ~100 held cap (drop-oldest) — explicitly so a message
  loop between two sessions stops on its own.

**Agent teams** (experimental, lead + teammates): file-based JSON inbox per
agent at `~/.claude/teams/{team}/inboxes/{agent}.json`, a shared task list
with file-locking for claim races, idle notifications from teammate to lead.
A known bug class worth stealing the fix for: pre-v2.1.207, one malformed
inbox entry blocked the entire mailbox — the fix was per-entry validation
with quarantine of bad entries.

**Trust rules** (the clearest deployed statement anywhere):

1. A message is labeled as *from another Claude session, not the user*.
2. It can never approve anything — a relayed approval claim is untrusted
   input, which closes the confused-deputy path (a denied agent cannot
   launder its request through a peer).
3. Slash commands in message text arrive as plain text, never executed.
4. The receiver's own permission rules still apply to whatever the message
   asks for.

## 2. What the field has converged on

- **Message passing beat shared state** for cross-process agents. LangGraph
  (shared typed state) and CrewAI (hub-and-spoke output passing) are
  single-process designs; AutoGen v0.4 rebuilt on an actor model; A2A —
  Google's Agent2Agent protocol, now Linux Foundation, having absorbed
  IBM's ACP in Aug 2025 — is the cross-vendor standard (Agent Cards at a
  well-known URL, `Task` lifecycle with eight states, poll + SSE + webhook
  push with poll as ground truth). MCP deliberately stays agent→tool; its
  Nov 2025 `Tasks` primitive converged on the same lifecycle vocabulary but
  its 2026 roadmap explicitly does not chase A2A.
- **Delivery semantics**: everyone lands on queue-until-safe-point, never
  interrupt; at-least-once with an idempotent receiver; a durable store as
  the truth with push as an optimization. Nobody delivers peer messages as
  system-role content — the norm is a user-role turn with explicit
  "not-from-the-user" provenance. (The no-legal-slot-between-`tool_use`-and-
  result constraint mecha's steering already handles is the same constraint
  everyone else hit.)
- **Security literature**: Morris-II (arXiv:2403.02817) demonstrated
  self-replicating prompts crossing agent boundaries; Prompt Infection
  (arXiv:2410.07283) showed LLM-to-LLM injection spreading across a network
  from one poisoned document, and measured that *sender-tagging helps but
  only combined with content-level defenses* — intermediate agents paraphrase
  injections into cleaner forms. Trustwave demonstrated Agent-Card
  description text prompt-injecting an LLM-based router. The consensus: an
  inter-agent message deserves at most the trust of the least-trusted input
  the sender ever read — and since the receiver can't verify that, it's
  untrusted-by-default with provenance attached.
- **The gap nobody has closed**: no deployed system forwards *taint* across
  the agent boundary. A message is a laundering point — the sender's
  exposure history is invisible to the receiver. Claude Code labels the
  sender but not what the sender had read. mecha is unusually well placed
  to close this, because taint is already a serialized, per-conversation
  value.

Concrete pitfalls with evidence, all worth designing against from day one:
message loops (real; caps + dedup), one bad entry wedging a mailbox (real;
per-entry validation), approval laundering (blocked structurally or not at
all), interrupt-style delivery corrupting the transcript (fold at the
results message), and assuming exactly-once from push (store is truth,
receiver idempotent).

## 3. What mecha already has

The published state of the art independently converged on several things
mecha already does — which is the strongest sign the additions are small:

- **Steering is the delivery mechanism.** `RunContext::queued_input`
  (agent.rs:220) drains at the top of each turn (agent.rs:869) and
  `append_user_text` (agent.rs:162) folds text into the message already
  carrying tool results. That *is* Claude Code's delivery semantics. Today
  the queue has exactly one writer (the TUI) and exists only while a run is
  in flight.
- **The frontdoor is the inbound-message pattern**: a directory of JSON as
  the seam, one file per record, states, atomic writes, unknown fields
  preserved, `reconcile` run on read rather than on a verb. What it lacks
  for messaging is an addressee — it's a single global queue for "the user".
- **Triggers are the cross-process signaling template**: `.running` marker
  with pid + liveness check (`process_alive`, trigger.rs:825 — with the
  range guard on the pid), the `.cancel` sentinel file the runner polls
  every 2s, flocks the kernel releases on death. Everything a session
  liveness registry needs, currently keyed by trigger name only.
- **The outbox is the egress review**, and staged sends already record a
  taint snapshot and a session id — the only path today by which a run
  knows its own identity (`OutboxRoute::set_session_id`, best-effort).
- **The taint machinery is the trust model** messages must plug into — with
  one crux: taint is only ever set from a `ToolOutput` (agent.rs:2082);
  `append_user_text` takes a bare `String` and touches no taint. A peer
  message folded in as steering today would enter the conversation
  completely unlabeled. HANDOFF already names this for webhooks: "the
  payload must arrive marked untrusted — the first time the interlock's
  rules applied to a *prompt* rather than a tool result."
- **The TUI already watches stores for other processes' effects**: the
  `Watch` enum polls stores (never children), the adaptive tick refreshes
  the outbox badge while idle, notices don't stack over modals.
- **There is no IPC anywhere** beyond the filesystem: no sockets, no
  watchers, no signals into runs (deliberately — the cancel file exists
  because SIGTERM would kill the daemon). Every cross-process observation
  is a poll.

Gaps, in the order they bite: no session liveness registry; a run cannot
learn its own id inside core (`RunContext` has no identity field, so no
tool can stamp a sender); no per-session inbox; no wake-up for an idle
session; no outside-in delivery into a running run; taint never set from
injected prompt text; no thread/correlation id; no capability axis for
"sends to a peer on this machine".

## 4. Proposed shape

**A file-based mailbox, polled at turn boundaries, with taint forwarded on
every message.** No sockets, no daemon, no watchers — the store is the
truth and polls are how this codebase observes other processes. A socket
buys sub-second latency mecha doesn't need (delivery waits for a turn
boundary anyway) at the cost of a second IPC idiom.

### The store

`~/.mecha/messages/<recipient>/` — one JSON file per message, following the
outbox's conventions exactly: pretty-printed, temp-sibling-and-rename, 0700,
`$MECHA_MESSAGES_DIR` override, advisory flock for state-changing writers,
*sending takes no lock* (a fresh message is a fresh file with a unique id).
Per-entry validation on read; a malformed file is quarantined and reported,
never allowed to block the inbox (Claude Code's pre-v2.1.207 bug).

```json
{
  "id": "<Session::new_id()>",
  "from": "<producer>",  "from_session": "<session-id>",
  "to": "<producer>",
  "body": "…",
  "reply_to": "<message-id or null>",
  "taint": { "private": false, "untrusted": true },
  "state": "pending | delivered | held | dropped",
  "created_at": "…"
}
```

**Addressing is the producer namespace** (`work.rs` validation rules —
already the one stable, human-writable identifier space): `chat`, a
trigger's name, a session id. A session registry generalizes the trigger
`RunMarker`: `~/.mecha/agents/<producer>.running` with pid + liveness
check, written by every front-end at run start. `mecha msg list-agents`
reads it; a message to a dead or never-alive producer still lands in the
store and waits — persistence is the point, liveness is advisory.

**The `from` and `taint` fields are stamped by the harness, never composed
by the model.** The model writes `to` and `body`; core fills sender
identity from the `RunContext` (which gains a `session_id`/`producer`
field — closing gap #2, and giving `ToolCtx` a sender to stamp) and the
taint snapshot from the live `Conversation`.

### Sending

A `message_send` tool (registered only when `[messages]` is enabled) plus
`mecha msg send --to <producer> "text"` for humans and scripts.

**`message_send` is not `external_send`, and taint forwarding is why.**
The write never leaves the machine — owner-only files under `~/.mecha`,
same user both ends — so the interlock's exfiltration rationale doesn't
apply. The real risk is *laundering*: private data or an injection hopping
to a peer whose interlock saw none of it. The answer is not to refuse the
hop (which would make messaging unusable exactly when a tainted overnight
run has something to report) but to make the hop carry its history:

- **Delivery ORs the sender's taint into the receiver's conversation.**
  A message from a run that had read mail arrives and sets
  `taint.untrusted`; the receiver's own interlock then governs any
  external send exactly as if it had read the mail itself. This is the
  invert-move: relocate the enforcement to the receiving side of the
  boundary rather than blocking the boundary.
- Unknown or missing taint (old message, torn write) classifies untrusted —
  fail closed, same rule as learning's `Origin`.
- The body is wrapped in the existing `<untrusted-content source="…">`
  labeling **when the sender's taint was untrusted**; a clean-taint message
  from your own harness under your own uid is labeled with provenance
  ("message from trigger `morning`, not from the user") but not as
  third-party content — the `external` vs `untrusted_input` distinction
  applied to peers. Passing through a model launders nothing (the subagent
  rule); passing through a *clean* model conversation had nothing to launder.

Loop control at the store, both enforced at write time: an identical
`(from, to, body)` while the original is still pending deduplicates (the
sender is told, nothing new is written — pending-ness is the window, so no
clock is involved), and a full mailbox (default 50 pending) **refuses** the
send rather than dropping the oldest. Drop-oldest — Claude Code's choice —
is silent loss of message one to admit message fifty-one, and the sender
here is an agent that can be told "full" and act on it.

### Delivery

Three receiver situations, one store:

1. **A running run** polls its own inbox at the top of each turn — the same
   site that drains `queued_input` — claims pending messages (flock,
   mark `delivered` before folding, so a crash re-delivers rather than
   double-delivers… at-least-once with the transcript as the dedup),
   merges taint, and folds the wrapped body via `append_user_text`. Same
   fold point as steering; new labeled path, not the bare-string one.
2. **An idle TUI session**: the existing idle tick grows an inbox badge;
   policy decides whether arrival starts a turn. `[messages] inbound =
   "accept" | "hold" | "refuse"`, default **`hold`** — a notice and a badge,
   the human fires the turn. Set only by config or slash command, never
   inferred from the prompt (the `/review` rule: admission policy must not
   be decidable by anything sharing a context window with third-party text).
   `accept` is the unattended-coordination mode; per-sender overrides can
   come later if wanted.
3. **No process at all**: messages wait. The next run of that producer
   (trigger fire, `chat` resume) delivers the backlog. A trigger is the
   existing wake mechanism if one is wanted; no new daemon.

**What a message can never do** (ported from Claude Code, mostly already
structural here): it is not the user — it cannot approve an approver
prompt, cannot change config, cannot count as consent; slash-command-shaped
text is content, not input (deliver through the fold, never through the
TUI input path); the receiver's own permissions, hooks, outbox route and
interlock govern everything the message provokes. `mecha eval` forces
messaging off, like MCP, hooks and the outbox, for the same
reproducibility reason.

### Config

`[messages]` on `Config` — **two edits** (Config + ConfigLayer, the
round-trip test enforces it). Global-only load for anything a peer message
can influence? No — the config here is receiver-side policy (`inbound`,
caps, enable), which is the trigger side of the trust line: a cloned repo's
`mecha.toml` must not be able to set `inbound = "accept"` on someone's
session. Load it like triggers: global file only.

### Phasing

1. **Store + CLI + turn-boundary delivery + taint forwarding.** `mecha msg
   send/list/show`, the registry file, `RunContext` identity, the fold-in
   with labeling and taint merge, caps and dedup. This alone gives the
   useful case: an overnight trigger messages `chat`; the morning session
   opens with it.
2. **TUI**: badge, notices, `/messages` modal on the `/triggers` pattern
   (store read for display, mutations via `mecha msg`), `inbound` policy,
   live delivery into an in-flight run.
3. **Later, if ever**: reply threading beyond `reply_to`, a synchronous
   ask-a-peer built on the `Asker` shape, A2A alignment for cross-machine
   (a different feature — that one *is* `external_send`).

### Deliberately absent

- **Sockets/watchers** — a second IPC idiom for latency nothing needs.
- **Cross-machine transport** — changes the trust math entirely (that's
  A2A's problem, and any such tool is an `external_send` sink).
- **Message-driven approval or config** — the confused-deputy door.
- **A "trusted peer" flag on messages the *model* composes** — trust is
  computed from recorded taint by the harness, never asserted by a sender.
- **Delivering history or files** — text only; a path in a message is a
  path the receiver's jail resolves, same as any other model-supplied path.

## 5. Adjacent unlocks

The infrastructure is more general than the feature that motivated it. By
piece:

**Run identity in core** (`RunContext` knows its session id / producer):

- Provenance stops being best-effort. Today the outbox learns its session id
  only because each front-end pokes it in after the fact (`None` for
  batch/eval). With identity in core, every staged draft, work artifact,
  ledger row and distilled episode is stamped by construction; frontdoor
  reconciliation and learning attribution get sturdier for free.
- Self-referential tools: "distill this session when done", "schedule a
  trigger that resumes *this* conversation" — "this session" becomes a
  concept core holds, not just the front-end.

**A session liveness registry** (the trigger `RunMarker` generalized):

- `mecha ps` — what is running, since when, in which workspace, which pid.
  Currently answerable only per-trigger.
- Generalized cancel: the `.cancel` sentinel mechanism, available for any
  producer, not just triggers.
- Collision warnings: "another live session is already jailed to this
  workspace" at startup (the shared-checkout collision, caught early), and
  `work clean` declining to sweep a live run's workspace.
- Safer and richer **resume**. Resume itself already exists (the session
  `Taint` record exists precisely so resuming doesn't launder taint), but
  the registry makes it safe and discoverable: refuse to resume a session
  whose marker shows a live pid (two processes appending to one JSONL
  transcript is corruption waiting to happen), and list resumable sessions
  beside running ones. And resume composes with the mailbox: a resumed
  producer's waiting backlog is delivered at the first turn boundary, so
  picking a conversation back up also picks up what arrived while it slept.

**The labeled delivery path** (outside text folded in with provenance +
taint merge — closing the "taint is never set from prompt text" gap). This
is what several backlog items were already blocked on:

- **Inbound webhooks.** HANDOFF names the hard part: the payload must
  arrive marked untrusted — the first time the interlock applies to a
  *prompt* rather than a tool result. A webhook becomes another writer
  into an inbox, taint pre-set untrusted.
- **File-watcher triggers.** Same path: a "what changed" payload delivered
  labeled at a turn boundary.
- **Headless steering.** Steering is TUI-only because it needs one stdin
  owner; an inbox polled at turn boundaries sidesteps stdin. `mecha msg
  send --to morning "skip the news section"` steers a trigger run
  mid-flight from any terminal.
- **Progress from unattended runs.** A trigger can currently only speak at
  the end, through `notify`. Mid-run reports land in the session you will
  actually open.
- **Notes to a future self.** A message to a producer with no live process
  waits and is delivered into context on that producer's next run —
  durable, taint-labeled, automatic.
- **Detached workers.** Registry + inbox is the substrate for
  spawn-and-detach: a long-running child reports back by messaging the
  parent's producer (Claude Code's background-task pattern from existing
  parts). Today a subagent is a blocking call with no identity.
- Possible with care: an **async `ask_user` relay** — a headless run parks
  a content question as a message. Content questions only; never
  permission approvals ("a message can never approve anything" is the
  confused-deputy line).

## 6. Decisions (settled 2026-08-09)

*Each of these was an open question; the "proposed" answers below were
reviewed and accepted on 2026-08-09. They are decisions now — don't re-ask
them, re-argue them in a PR if the code proves one wrong.*

*A high-effort multi-agent code review the same day found ten confirmed
defects in the first cut, all fixed before merge. The ones that changed a
decision here or added an invariant:*

- ***Subagents cannot send.*** *The taint stamp is a per-turn snapshot of
  the sending run's own conversation, so a subagent's `message_send` would
  label the message with the child's fresh (clean) taint or a frozen parent
  snapshot — either way a laundering path around the whole point of §"taint
  travels". `message_send` is now refused to subagent registries outright
  (a profile asking for it is a hard startup error). The parent sends, based
  on the child's returned prose.*
- ***The CLI fails closed.*** *`mecha msg send` stamps clean taint only when
  stdin is a real terminal (a person typing — the one trusted sender); a
  pipe, a script, or an agent's `shell` shelling out to it gets untrusted,
  closing the "one tool over from the guard" hole `shell` + `sandbox="none"`
  opened.*
- ***`message_send` is refused during `Phase::Plan`.*** *It is `read_only`
  for the approver but side-effecting for the phase gate, which keys on
  `read_only`; the refusal is explicit in `call` rather than flipping the
  flag (which would drag the approver back in and break the unattended
  shape).*
- ***Delivery is skipped when the run is about to stop.*** *Claiming marks a
  message delivered irreversibly; a run at its turn/budget ceiling or loop
  guard would consume mail it never acts on. The mailbox fold is now gated
  on the same stop condition the loop already computes.*
- ***`claim_pending` returns what it committed.*** *A write failure partway
  through hands back the messages already marked delivered (so the caller
  folds them) and leaves the rest pending, instead of erroring the whole
  batch to nothing — the earlier version could strand a delivered-on-disk
  message that reached no conversation.*
- ***Inbound default keys on `global_config_only`, not `interactive`.***
  *Only the trigger runner (the sole setter of `global_config_only`)
  defaults to `accept`; everything a person drives — including a piped
  `run --json`, which is unattended for *approvals* but must still hold mail
  — defaults to `hold`. `interactive` was conflating two different
  questions.*

*Smaller review fixes, no decision change: a transient IO error skips a
message this scan rather than quarantining it as corrupt (only a parse
failure is `.bad`); the duplicate brake keys on `reply_to` too, so the same
body answering two different threads is two messages; `inbound = "refuse"`
warns at startup that it behaves as hold; and the project-layer strip has a
test.*

1. **Producer collisions.** `chat` is one producer but the workflow runs
   several concurrent sessions (worktree per session). If the mailbox is
   keyed by producer, which live `chat` drains it? Proposed: a mailbox
   *is* a producer; any live run of that producer may claim (flock makes
   the claim exclusive per message); session-id addressing exists for
   precision when it matters. The same collision applies to the `.running`
   marker — it must be per-session, listed under the producer, not one
   file per producer name.
2. **Claim ordering.** Mark-delivered-then-fold is at-most-once (a crash
   between loses the message); fold-then-mark is at-least-once (a crash
   re-delivers). Pick one and write it down. Proposed: claim (rename into
   the session's name) → fold → mark delivered; a claimed-but-unmarked
   message from a dead pid is returned to pending.
   *As built (simplification of the same intent):* there is no rename
   phase — the whole claim (read pending, mark `delivered` with
   `delivered_to`) happens under the recipient's flock, and the fold is a
   synchronous in-memory push in the same thread immediately after, so the
   crash window between mark and fold is microseconds and a dead claimant
   cannot wedge anything (the kernel releases the flock). The trade
   accepted: a crash *after* the fold, before the session file is written,
   leaves the message `delivered` but in no transcript — and recoverable,
   because the store keeps the full body and names the session that died.
   Losing a message silently is the worse failure, and nothing is ever
   only in a transcript.
3. **Taint persistence timing.** Message-borne taint must reach the
   session file's `Taint` record, not just the in-memory conversation —
   resuming after a tainted delivery must not launder it. Delivery happens
   inside core; today taint records are written by front-ends. Decide who
   writes it and when.
4. **Per-surface `inbound` defaults.** `hold` is right for the TUI (badge +
   human). But a trigger run has no human, and `hold` there means headless
   steering never works until config changes. Proposed: attended surfaces
   default `hold`, unattended runs default `accept` for delivery-into-run
   (their permission mode is already read-only by default, and the taint
   merge governs what a message can provoke). Needs a deliberate decision,
   not a fallthrough.
5. **Body size cap.** Text only, but a cap (and the refusal wording when
   exceeded) should exist before the first megabyte message, not after.
6. **Authorisation.** Same-uid may message same-uid, unrestricted, in v1.
   Say so explicitly so a later allowlist is an addition, not a breaking
   change.

## Sources

- Claude Code: cross-session messaging, agent teams, subagents docs
  (code.claude.com/docs); Agent SDK sessions & streaming-input docs.
- A2A spec (a2a-protocol.org) and the Linux Foundation ACP merge
  announcement; MCP Nov 2025 Tasks (SEP-1686) and the 2026 MCP roadmap.
- Morris-II, arXiv:2403.02817; Prompt Infection, arXiv:2410.07283;
  CaMeL, arXiv:2503.18813; multi-agent security survey, arXiv:2505.02077;
  agent-interoperability survey, arXiv:2505.02279; Trustwave SpiderLabs
  agent-in-the-middle writeup; CSA MAESTRO A2A threat model.
- In-tree: agent.rs (steering, taint, interlock), frontdoor.rs, trigger.rs
  (markers/sentinels), outbox.rs (store conventions), work.rs (producer
  namespace), docs/HANDOFF.md (webhooks/watchers not-built notes).
