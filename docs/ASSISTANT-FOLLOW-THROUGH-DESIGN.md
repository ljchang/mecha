# Personal assistant follow-through

2026-09-08. This design connects delegated work, review, delivery and verification
without moving the task board out of the graph or weakening the owner boundary.
Implementation authorized by the owner after the harness review.

Merged as PR #216 and installed from `main` at `c3f33f4c` on 2026-09-08; see
`HISTORY.md` and the verified update in `HANDOFF.md`. The first implementation
accepts commitments through owner commands, produces in-app reminders, and verifies
explicit workflow criteria. Automatic promise extraction (proposed follow-ups: §Model-proposed follow-ups),
external push notifications and a general in-loop convergence engine remain
future extensions. Tool profiles are opt-in with `--tool-profile`; capability guidance also accompanies
any configured outbox route so a minimal install knows how to stage drafts.
The daily priority list joins workflows, drafts and questions; existing dashboard
links still lead to the broader mail, calendar and board surfaces. Owner-action
counts measure requested verbs, not human time. Restart/ambiguous-failure scenarios
are deterministic tests; the model lifetime covers sequential reply/calendar work
and injection resistance.

The grounding follow-up adds explicit fixture time per task and rubric checks
against actual tool evidence. See `HISTORY.md` for the calibrated live results.
The rubric judge can detect unsupported claims, but is an evaluation tool rather
than a runtime truth guarantee. Mail rows name the owner-mailbox scope and unknown
recipient read status; ordinary assistant guidance limits claims to read sources.

## Decisions

1. Preserve global outbox routes and review-store identity across project layers.
2. Keep per-result provenance in local transcript metadata, never provider payloads.
   Old recordings remain unknown; replay conservatively treats unknown as external.
3. Before any remote release, durably record the delivery attempt. A crash or
   ambiguous failure leaves outcome unknown and blocks automatic resending.
   Reconciliation records evidence of delivery or non-delivery; a local record
   alone cannot promise exactly-once effects on an arbitrary remote API.
4. Core owns a workflow journal referencing the existing graph task, transcript,
   question and outbox IDs. It records checkpoints and partial effects; the graph
   continues to own tasks and the owner continues to close them.
5. Verification is evidence against explicit postconditions. It distinguishes
   pending review, failed, unknown and verified; a model stopping is not proof.
6. Commitments record source, responsibility, due time, next check and resolution.
   Model extractions are proposals until accepted. Follow-up is deterministic,
   deduplicated, respects quiet hours and produces a digest for the owner.
7. Structured output is a provider-neutral request capability with explicit failure
   when unsupported, adopted first for quarantined extraction and classification.
8. Tool profiles narrow a stable registry. Capability-dependent prompt text comes
   from the registry and cannot advertise unavailable tools or taint escape routes.
9. The daily view joins existing stores around urgency, decisions, completed work
   and waiting. It reports missing sources as unknown and explains ordering.
10. Assistant evaluations check final fixture state across changing plans,
    interruptions and failures, with repeated trials and owner-effort measurements.

## Verification and rollout

Each correctness fix has a regression reproducing its prior failure. Stores are
version-tolerant, locked for mutations and atomically persisted. Tests cover restart,
ambiguous delivery, duplicate action, stale evidence and denied approval. Provider
encoding tests and fixture-backed assistant evaluations supplement scripted loops.
The final checks are workspace build/test, formatting, Clippy, frontend tests and
build, documentation updates, and the repository handoff procedure.

The implementation phase did not deploy services. The owner subsequently authorized
the 2026-09-08 installation recorded in `HANDOFF.md`; no release was published.
Native provider idempotency is used only where its contract is known; unsupported
effects retain the reconciliation boundary.

## Model-proposed follow-ups

2026-11-18. Decision 6 left "automatic promise extraction/proposal acceptance"
for later; this is that extension, built on the workflow store rather than
beside it. The owner asked for it after reviewing always-on agent designs:
the assistant should be able to say "I'll check back on this tomorrow" and
mean it.

**Rulings** (the owner, 2026-11-17 and 2026-11-18):

| # | Ruling |
|---|---|
| F1 | An accepted follow-up, when due, **wakes a run**, not only a reminder. This amends the rule that a workflow tick never starts an agent; the woken run continues the conversation the follow-up came from, inherits its taint, and can only stage. |
| F2 | **The owner accepts** in the Review tab's proposals pane. In auto review mode an untainted proposal is accepted automatically — the drafts rule, `review_policy::auto_releases` — and the owner intends to relax this once trust is built. |
| F3 | **The answer goes where the follow-up was proposed.** On the web that is the chat; the way back to it is the workflow's reminder, on Home and in Tasks → workflows, with an Open chat link. No new tab. |
| F4 | **Web first.** The TUI and Slack follow. |
| F5 | **It has an off switch** (2026-11-18): `[features] followups`, the Settings → Features toggle, off until the owner turns it on; `[tools] disabled` withholds the tool too. Off stops new proposals; decided and waiting ones stay. |
| F6 | **Voice calls propose too** (2026-11-18): a spoken turn in a web chat and the voice facade's own slot both name their conversation, so a follow-up from a call returns to it. |

**The shape.** A run calls `propose_follow_up` with one line and a time; the
tool writes a record to `~/.mecha/followups/` (`followup.rs`) carrying the
conversation's session, jail and taint, and returns. Nothing is scheduled.
`mecha followups accept` — the proposals pane drives it — is the only writer
that turns a proposal into a workflow: the session it returns to, and a
commitment with `follow_up_at` and no deadline, so it is never overdue and
raises the existing "Follow-up due" reminder at its time. The workflow
store's rule that no model tool mutates it is unchanged; the model writes
only proposals, and acceptance crosses the owner.

- **A proposal needs a recorded conversation.** The tool reads
  `ToolCtx::recorded_session` — stamped on the agent's context by the front
  end that records the conversation (the web chat per turn, the TUI per
  agent build, so a switch keeps it), never derived from the outbox route —
  and refuses where there is none: a batch, an eval, an incognito chat.
  Voice calls name theirs on both doors (F6): `begin_turn` stamps a spoken
  turn like a typed one, and the facade slot stamps its own recorded
  session.
- **Taint is recorded fail-closed** and shown at decision time: a proposal
  from a conversation that had read outside content is an injection's way to
  schedule itself, so `show` warns and auto-accept (F2) never takes one.
- **The words are bounded** to one line of 200 characters, the time to
  between ten minutes and ninety days out.
- `forget` removes a conversation's proposals with it.

**Build order.** Each step is its own pull request.

| Step | What | State |
|---|---|---|
| 1 | The store, the tool (web chat and TUI), `mecha followups`, the proposals pane, the Home card, Open chat on a workflow | merged (#896) |
| 2 | F1: a due follow-up wakes a staging-only run in its conversation, run by the host that owns it; its answer lands in the chat and the workflow's reminder says it was answered | merged (#897) |
| 3 | F2's auto-accept, on the surfaces that have an auto mode | after 2 |
| 4 | The acted-on measure: whether the owner opened, replied in or acted on the chat within a day of a woken answer | this change: `mecha followups measure` (§Step 4); "opened" is not measured |

### Step 2: a due follow-up wakes its conversation

F1 amends the workflow rule that "nothing here automatically starts an agent".
It is amended for one case and on these terms; every other workflow reminder
still only reminds.

- **`mecha serve` wakes, and nothing else does.** A web chat's conversation
  lives in serve's memory, and a second writer on its transcript forks it —
  the hazard `questions answer` documents as D11. So the waker is a task
  inside serve, beside the late-delivery task, and the turn it starts goes
  through `begin_turn`, as a voice call's does. The trigger tick keeps
  raising the in-app reminder and never starts a run. A follow-up proposed
  in the TUI is a recorded session too, and serve wakes it the way
  `#chat/<id>` resumes one; the TUI never wakes anything.
- **What it wakes.** An accepted follow-up's workflow whose `follow_up_at`
  has come, that is open, not snoozed, not running, and has not been woken
  before (`woken_at`, a new optional field, written before the turn starts so
  a crash never runs it twice).
- **A few at a time.** One pass wakes at most three (counting turns started,
  not candidates looked at, so follow-ups that wait never starve the rest),
  oldest first, so what
  accumulated while the outbox was unset or serve was down comes over the
  next minutes rather than as one burst of unattended turns.
- **When it does not, and says so.** Three answers, and the reminder stands
  in every one:
  - *Never, recorded:* a memory chat (typed input only). The one refusal that
    retires the follow-up.
  - *Waiting, recorded once and retried each minute* (`follow_up_waiting`),
    because the owner can change it: no outbox route (a woken run that could
    send directly is not staging-only, so configuring `[outbox] tools` later
    wakes what came due meanwhile); a transcript that cannot be found or
    classified; a turn that could not start, whose claim is given back.
  - *Later, silently, next minute:* the feature is switched off (a pause);
    another writer holds the conversation (a task run, the TUI); a turn is in
    flight; the transcript ends on the owner's own words (a tail of tool
    results takes the wake, folded in and marked the harness's); no
    background seat is free.

  The woken mark is claimed under the workflow store's lock, re-checking the
  whole rule there, so a second `serve` on one home never wakes the same
  follow-up and a workflow closed or snoozed since the listing is not woken.
- **The turn.** A harness-voiced user turn, with its own stem so the
  correction miner and the taint reader know it is not the owner's:
  `(From the harness: a follow-up you proposed is due — "<what>", proposed
  <when>. Check it now and tell the owner what you found. Anything you would
  send is staged for their review.)` The words in quotes are the model's
  own, from this conversation, and carry its taint; the turn adds none and
  launders none. Unattended: a call that needs an approval is refused in the
  machine's voice, as `questions answer --unattended` does; routed sends
  stage. Staging-only is enforced per tool, not by a route existing: every
  tool that can send anything (`Egress` other than `None`) that `[outbox]
  tools` does not route is withheld from the woken turn — `http_fetch` is
  read-only, so no approver would ever be asked about it, and `web_search`
  declares `Blind` while a deep search reaches a chosen backend. `shell`
  goes too wherever its sandbox can reach the network, so the canonical
  "check whether the build passed" needs a network-isolating sandbox or a
  routed tool to answer it. `ask_user` parks instead of offering a card:
  nobody is there to answer one (§Unattended runs ask, below; it was
  withheld until 2026-11-19).
  Its run posture is `unattended` too, so a shell it runs is never
  read as the owner's approval (`closure::cause`). It takes a background
  seat (`permit.rs`), like any unattended run.
- **After.** The answer is in the chat. The workflow records
  `follow_up_answered` (or `follow_up_failed` with the reason) and raises a
  notice, "Follow-up answered — open the chat", through the ordinary
  coalesced, quiet-hours-respecting path, so Home's workflow count and the
  card's Open chat button are the way back (F3).
- **Not here.** Auto-accept (step 3), the acted-on measure (step 4), Slack.

### Step 4: the acted-on measure

A sensor, under the owner's sensor-first ruling for proactive messages
(2026-11-17): `mecha followups measure [--json] [--since <RFC3339>]`
(`followup::measure`) counts, and **nothing reads it yet** — no surface, no
gate, no prompt. For each follow-up whose workflow records
`follow_up_answered`, it classifies what the owner did from the woken turn's
claim (`woken_at`) to 24 hours after the answer, from records only:
**replied** (the owner's own words, `agent::owner_text`, in a user message
after the wake — a steer into the woken run counts, a harness turn or a later
wake does not), **released** or **rejected** (a draft staged from that
conversation since the wake, resolved inside the window by
`Actor::Owner`), **dismissed** (the reminder's Dismiss, `workflow ack`, which
now records `notice_acked` with who ran it — bookkeeping, so it neither
re-raises the notice nor clears a verification), **none** once the window
has closed, **pending** before. Acted-on rate = (replied + released +
rejected + dismissed) / (those + none), a dash over an empty denominator.

- **Unknown is its own outcome, on neither side of the rate**: a transcript
  or outbox that cannot be read whole, a woken turn the transcript no longer
  holds, a draft or dismissal by a hand that is not provably the owner's
  (`owner-approved`, `unknown` — `forecast::observe`'s rule), or a reply that
  cannot be placed. A message has no time of its own; a reply is placed by
  the run ends recorded around it (`RunStats::ended_at`), and counted only
  when the run it belongs to ended inside the window. One provable act
  decides the follow-up whatever else could not be read.
- **"Opened the chat" is not measured.** Nothing server-side records a page
  being looked at, and the readout says so rather than inferring it.

## Unattended runs ask, and resume as a chat

2026-11-19. A delegated task run can already park a question and end
(`questions.rs`, D13): the owner answers later and the run resumes. Two other
kinds of unattended run could not ask at all, so they guessed or gave up: a
scheduled trigger (no `ask_user` in its registry) and a woken follow-up
(`ask_user` withheld, since its card would hold the turn's seat for ten
minutes with nobody to answer). This extends the parking to both.

**Rulings** (the owner, 2026-11-19):

| # | Ruling |
|---|---|
| A1 | **Triggers and woken follow-ups may park a question.** Other background runs (mail sweeps, rumination) may not. |
| A2 | **Answering is done in Tasks → Waiting; the run resumes as the owner's turn in its chat**, tracked from Tasks, its conversation one tap away (Open chat), never opened automatically. The result, any approval card and the rest of the conversation happen in that chat, not in the background. (Revised by the owner the same day: Tasks is the hub for background work; the first wording had the page open the chat.) |
| A3 | **An unanswered question never expires.** It waits until the owner answers or abandons it; `mecha doctor`'s existing staleness finding still names it. |

**The shape.**

- A question records how it resumes: `resume = "background"` (a delegated
  task run's, as today: `mecha questions answer --unattended` in a detached
  child) or `resume = "chat"` (A2). Absent on the wire is `background`, so
  every question parked before this reads as it always did.
- **A trigger run** registers `ask_user` over a `ParkingAsker` on its own
  recorded session, `resume = "chat"`. Asking stores the question and ends
  the run; the trigger's ledger records the run as waiting on the owner, with
  the question's id, rather than as finished.
- **A woken follow-up's turn** gets the same asker (the web chat's
  `ask_user`, told this turn is unattended, parks at once instead of offering
  a card) in place of the withholding. Its workflow records
  `follow_up_asked`, and the tick turns that into the owner's notice.
- **Answering a `chat` question** is done in Tasks → Waiting, and goes
  through `mecha serve`, never a detached child: the conversation is resumed
  into the session map (`resume_locked`, by the owner, so an archived chat
  comes back to the rail) and the answer starts an ordinary owner's turn
  through `begin_turn` — the chat's own approver, its own mode, steerable —
  so the run's history stays in one conversation. The page stays on Tasks:
  the answer's reply carries the chat's key, and the Waiting view shows the
  question as answered and continuing, with an *Open chat* link; nothing
  navigates by itself. The turn begins under the question store's lock and
  the question closes only once it has (`QuestionStore::answer_with`), so two
  answers racing for one question start one turn and a turn that cannot
  begin leaves the question open. The question is marked answered with the
  owner's words. A chat question answered from the terminal says to answer
  it in the web app, and is not consumed.
- **A trigger's preamble** (`UNATTENDED` in `commands/trigger.rs`) used to
  say a question was a dead end. It now keeps the reasonable assumption as
  the default and names `ask_user` only for a decision the run cannot make
  without the owner, saying the run ends there; without that the tool would
  be registered and talked out of use.
- **What the owner is told.** A trigger's `notify` is handed "a question is
  waiting — answer it in Tasks → Waiting", the question, and where the run
  got to, never the run's last words as if they were its answer; its ledger
  row is `waiting` with the question's id. A woken follow-up's workflow
  records `follow_up_asked` with the question's id, and the tick's notice is
  "Follow-up has a question for you — answer it in Tasks → Waiting".
- **Taint rides with the conversation.** The resumed chat carries its
  recorded taint, and the question's own taint warning (the one shown before
  any parked question is answered) still shows: an unattended run that read
  outside content asks well-formed questions too.
- Nothing about the trigger's or the follow-up's own reach changes: a woken
  turn stays staging-only until it parks; the owner's answering turn is the
  owner's, at the chat's mode.
- The parked question reaches the owner where questions already do: Tasks →
  Waiting and Home's Questions card.

**Known limits** (review of #904, left for the owner):
- A chat question is answerable only through `mecha serve` with chat on;
  `mecha questions answer` refuses it. A machine running triggers without the
  web app can park a briefing's question it has no door to answer.
- A turn that parks two questions in one round (tools run concurrently)
  records the first in the ledger row and the workflow event; the second is
  open in Tasks → Waiting with no row pointing at it.
- A chat resumed cold opens read-only, so an answering turn can read and
  stage but not write until the owner changes the chat's mode.
- A second `ask_user` in the owner's answering turn goes up as a card in
  the chat, as any owner turn's does, while the owner may still be on Tasks;
  it declines after the card's timeout. Parking it into Tasks → Waiting
  instead would change how a live chat asks, which is the owner's call.
- An answer waits for the asking trigger's run to finish writing: the
  question names its trigger, and serve refuses while that trigger's running
  marker stands. The resumed chat also merges the question's own taint, so a
  transcript that lost its taint checkpoint never resumes clean.
- A carrying trigger carries a `waiting` run's transcript like an `ok` one:
  answered, its last words are the answer; unanswered, where it got to.

**Not here:** a Slack or TUI answer path for chat questions; parking from
other unattended runs (A1).

## Tasks → Background: every unattended run in one place

2026-11-20. The owner's framing (2026-11-19): **the Tasks tab is the hub for
background work** — where a background run's questions are answered, and
where every background run is tracked and monitored. Questions already land
in Tasks → Waiting (§Unattended runs ask). What was missing is the second
half: a scheduled trigger's runs appear nowhere in the web app, a woken
follow-up shows only as a line on its workflow card, and nothing says
"running now / waiting on you / done / failed" across them.

**Ruling** (the owner, 2026-11-20):

| # | Ruling |
|---|---|
| B1 | **Both, jobs as status lines.** Assistant runs are full rows with *Open chat*; below them, each scheduled job (the machine's `mecha-*` timers — the mail sweep, rumination, the GitHub collector, dashboard refreshes) is one line: last run, ok or failed, next run. |

**The shape.**

- **A view, not a verb.** Tasks → Background lists and links; it answers,
  stops and dismisses nothing. Answering stays in Tasks → Waiting, stopping a
  delegated run stays on its board card, dismissing stays on the workflow
  card — one door per act, so the view cannot drift from the surfaces that
  own them.
- **Assistant runs, newest first, from three stores:**
  - *Scheduled triggers* — the trigger ledger (`TriggerStore::runs`) over
    the last seven days, plus a run in flight from the trigger's running
    marker. State: `running` (live marker), `waiting` (`RunStatus::Waiting`;
    the row links its question in Waiting), `done` (`ok`), `failed`
    (`error`), and `unknown` for a status this build cannot name — never
    shown as done.
  - *Woken follow-ups* — workflows whose source is `followup:` and which
    have been woken (`woken_at`), read from their events: `follow_up_woken`
    with no outcome yet is `running`, `follow_up_asked` is `waiting`,
    `follow_up_answered` is `done`, `follow_up_failed` is `failed`.
  - *Delegated task runs* — the board's own `run` summary per task (the page
    already holds it from `/api/tasks`), so the view adds no second reader of
    the graph store.
  - Every row: what it is (trigger name, follow-up wording, task title),
    when it started and ended, its state, and *Open chat* to
    `#chat/<session>` — the session id, which the chat route resumes (the
    door #904 settled). A row whose run read outside content carries the
    same taint badge the question card does.
- **Scheduled jobs**, one line each, from systemd: `systemctl --user
  list-timers 'mecha-*'` for last and next, and the service's `Result` /
  `ActiveState` for ok, failed or running. Read in one bounded call per
  page load, never per row.
- **Unknown is said, never zero.** A store that cannot be read is a line
  saying so in its own section, not an empty list; on a machine without
  systemd (a Mac runs these under launchd) the jobs section says it cannot
  read them there, never "no jobs".
- **One read-only endpoint** (`GET /api/background`) for triggers,
  follow-ups and jobs; each section carries its own error so one unreadable
  store does not blank the others. The page polls it only while the view is
  open.
- Nothing here changes what any run does or may do; it is a read.

**Not here:** stopping a trigger run from the view; run history beyond seven
days (`mecha trigger show <name>` lists a trigger's recent runs); Slack.
