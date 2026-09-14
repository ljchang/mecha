# Approving by voice — research

Researched 2026-09-13. The question: **why can't the owner review and approve
inline during a call, and what would make it work?**

**Status.** §4.1, §4.2, §4.3 and §4.6 built 2026-09-14 on
`voice/spoken-approval` (`review_policy::segment`,
`Confirmations::carry_unanswered`, `ToolCtx::review_hint`,
`voice::SPOKEN_REVIEW_HINT`, two sentences in `VOICE_BLOCK`). §4.4, §4.5,
§4.7 and §5 are not built.

The short answer is that there are two different things behind "approve", and
the one the owner actually hit today is the one that already exists and
failed. Section 1 is the incident, read from the worker's journal. Section 2
is why it failed — three mechanisms, each sufficient. Section 3 separates the
two halves of the question and measures how much of each there is. Section 4
is the fix for the half that exists; section 5 is the design for the half
that does not. Section 7 is the recommendation.

Everything cited by symbol; the commit this was read against is `97c57544`.

## 1. What happened at 19:36 on 2026-09-13

`journalctl --user -u mecha-voice-worker`, one call, times UTC. The worker's
journal is the instrument here rather than serve's: `mecha serve` runs at
the default `warn` filter and every line the confirmation path writes is
`info`, so serve's journal for this window is `-- No entries --`.

```
19:36:08  heard   "Can you add a recurring meeting from twelve to two for my lab meetings on Wednesdays."
19:36:32  said    "Let me check your calendars first."
19:36:37  said    "Let me create the first lab meeting for this Wednesday, September 16th."
19:36:38  said    "That's drafted."                                    ← item 20260913T193636 (lab calendar)
19:36:47  heard   "Um yeah, I don't want that one. I want my Luke Chang Gmail calendar."
19:36:55  said    "Got it — let me scrap that draft and put it on your Gmail calendar instead."
19:36:57  said    "That's drafted on your Gmail calendar for this Wednesday from noon to two."  ← item 20260913T193654
19:37:02  said    "Want me to create more individual Wednesdays too?"
19:37:03  offer   "Here it is, in full. Title: Lab Meeting. Start time: Wednesday September 16 at 12 PM.
                   End time: Wednesday September 16 at 2 PM. Account: personal.
                   Defaults: all day false, calendar id primary.
                   I had read outside content when I wrote this, so listen to the addressing.
                   Say yes to send it, or later to leave it in your outbox.
                   That one is from your personal account."
19:37:44  heard   "Go ahead and send it."          (rms 0.0357, over_speaker=True)
19:37:45  said    "I don't have the ability to release drafts — that's your call.
                   You'll need to review and send it through `mecha outbox`."
19:37:51  heard   "I agree."
19:37:53  said    "Go ahead and release it whenever you're ready through your outbox."
19:37:59  heard   "Why can't I release it through you?"
19:38:02  said    "I can stage drafts, but the release step is intentionally yours — it's a
                   safety gate so nothing goes out without your review. I don't have a tool to bypass that."
19:38:17  heard   "But you did, you could do inline approvals."
19:38:22  said    "You're right, I should have been upfront about that. I genuinely can't release
                   drafts — I don't have that tool. I can only stage them for you to review."
19:40:33  outbox  20260913T193654 released from a screen — 2 min 49 s after the spoken yes.
          outbox  20260913T193636 (the lab-calendar draft the model "scrapped") is still pending.
```

Three facts to hold onto:

- **The offer played, in full, and the owner answered it.** Until today
  `VOICE-RESEARCH.md` recorded that thirty days of journal contained no
  *"Say yes to send it"* — the fourth door had never run in anger. It has
  now, once, and the first real answer to it fell through.
- **The harness never saw "Go ahead and send it." as an answer.** It reached
  the model as an ordinary turn, and the model — which has no release tool,
  by design — said what its tool result had told it: *review it with `mecha
  outbox`*. Every sentence after that is the model defending a rule it has
  been told and cannot see around.
- **The listener waited about forty-eight seconds to answer.** Speech ran
  continuously from "Got it — let me scrap that draft" (19:36:55) until the
  `over_speaker=True` flag on the owner's answer at 19:37:44 — the model's
  reply, its own trailing question, and then the readback with its defaults
  clause, taint line, menu and identity tail.

## 2. Why the spoken yes fell through

### 2.1 The accept vocabulary is a closed list of whole utterances, and it is narrow

`review_policy::parse_answer` normalises the utterance and requires it to
*equal* one of twenty `SEND_PHRASES`. The safety argument is right and is
recorded in its docstring: a substring rule would read "yes" out of *"yes,
but change the time first"* and send the draft the speaker was about to
correct. But whole-utterance equality against a flat list means every
natural *composition* of two accepted phrases is rejected. Run today's
`normalise` + `SEND_PHRASES` over twenty plausible spoken accepts:

| utterance | result |
|---|---|
| Go ahead and send it. | **drop** — both halves are in the list; the conjunction is not |
| Yes, go ahead. | drop |
| Sure, send it. | drop |
| Yep, do it. | drop |
| Okay, go ahead and send it. | drop |
| Yes please send it. | drop — `please` is stripped only from the tail |
| Send it now. | drop |
| Approve it. | drop — `approve` is in; `approve it` is not |
| Yeah send it | drop |
| Looks good, send it | drop |
| I agree. / Sounds good / Fine / Go for it | drop |
| Yes, but change the time first. | drop — **correctly** |
| Do it. | accept |

One accept in twenty, and the one correct rejection. The list was built by
hand from what a designer imagined saying, and the first live speaker said
a phrase made of two entries joined by "and".

### 2.2 A non-answer *drops* the question, and nothing ever asks it again

`confirm::react` returns `Reaction::PassToModel` for `NotAnAnswer`, and
`answer_completion` had already `take`n the `Pending` out of
`shared.confirmations` — the module docs call this out as a decision: *"An
unanswered offer is dropped, not remembered … the alternative — a question
that survives until answered — turns every later 'yes' in the conversation
into a live release."* That reasoning is sound against a *standing* yes. It
is not sound against the case it produced today: the owner's very next word
was an attempt to answer, and there was no question left to answer.

Nor does the next turn re-offer it. `offer_for_turn` offers only drafts
staged *since the turn's baseline* (`review_policy::staged_since`), and the
baseline is every pending id before the turn — so a draft that was offered
once and dropped is in every later baseline and is never spoken of again.
The only remaining paths to it are the page and the CLI, which is exactly
what the model went on to say.

### 2.3 The model was told the wrong surface, and it repeated it

The staged tool result in `agent.rs` reads *"The user will review it with
`mecha outbox` and release or reject it."* That sentence is composed in
`mecha-core`, which does not know which surface is listening, and it is
true on every surface but this one. The D10 `VOICE_BLOCK` tells the model
the harness *"reads the draft back word for word and asks whether to send
it"* — but nothing tells it what to say when the owner asks *how* to send,
so it answered from the tool result. It then said, correctly, that it has
no release tool; the deliberate absence of `outbox_approve` from the model's
surface (`mecha review`'s oldest rule) became, from the listener's side, the
assistant refusing.

The second untruth is the model's, and it is structural too: *"let me scrap
that draft"* — it cannot. It has no way to withdraw a draft its own run
staged, so the lab-calendar item is still pending, offered to nobody, with
the model having announced it gone.

### 2.4 A fourth thing, which did not bite today but will

The offer is spoken *after* the model's reply, in the same breath. Today
the reply ended with the model's own question — *"Want me to create more
individual Wednesdays too?"* — and the harness's question followed it. A
bare "yes" there is a yes to whichever the listener was answering, and the
harness takes it as a send. The D10 block should forbid a trailing question
on a turn that staged something; it does not yet.

## 3. Two halves, measured

"Review and approve inline" covers two mechanisms that are different in
kind:

**Drafts** — every call in `[outbox] tools` (mail, calendar, docs, factory
publishes). These are already *reviewed* by voice: the fourth door reads
the draft back from the store and matches the answer before any model sees
it (`voice/confirm.rs`, `VOICE-RESEARCH.md` §the fourth door). Today's
failure is in the answer half of that mechanism.

**Tool approvals** — every other non-read-only call: `fs_write`, `fs_edit`,
`shell`, `mail_triage`, `kg_upsert`, `kg_task_create`… These run with the
approver **off** on a spoken turn (`mecha serve --voice-yes` →
`TurnOpts::approve_all` → `ModeApprover { Allow }`), and the reasons an
approval cannot be asked mid-run are structural:

- `WebApprover::ask` raises a `WireEvent::Question` on the page's channel;
  `voice::pump` consumes `AgentEvent`, which has no approval variant, and
  discards everything but `TextDelta`. The caller hears keepalive pings.
- The transport is one `POST /v1/chat/completions` per utterance, with no
  side channel; the only way more words enter is a new request, and
  `VoiceHost::speak` defines a request against a live run as barge-in —
  `ws.questions.drain()` **then** cancel, because a run parked on a card
  never sees the token. Answering and interrupting are the same act.
- Without `--voice-yes` the failure is `APPROVAL_TIMEOUT` (120 s) of dead
  air, then `Blocked("nobody answered … nothing is watching this run")`.

How much of each is there? Over the 538 session files in `~/.mecha/sessions`
(read 2026-09-13; a spoken turn is a user message opening with the
`VOICE_BLOCK`):

- **51 sessions carry spoken turns, 56 spoken turns in all.**
- Tool calls in those sessions after the first spoken turn: 18
  `calendar_list_events`, 11 `web_search`, 9 `kg_search`, 8
  `calendar_create_event`, 7 `kg_entity`, 5 `kg_task_create`, 5 `docs_list`
  … and then singletons. **Non-routed writes: 6 across the whole corpus**
  (5 `kg_task_create`, 1 `fs_write`) plus 2 `research` delegations.
- **Outbox items staged by sessions with spoken turns: 12** — 11
  `calendar_create_event` (4 sent, 5 rejected, 2 pending, both pending ones
  from today), 1 `calendar_delete_event` (rejected), 1 `bundle_publish`
  (sent). Every one was resolved from a screen.

So the demand is lopsided: twelve drafts against six ungoverned writes. The
annoyance the owner named is the drafts half, and it is the half that
already has a mechanism.

## 4. Fixing the half that exists

Six changes to the fourth door, in the order they pay. None alters the rule
that carries it — *the harness asks and the harness hears; no release
decision enters a context window* — and each is testable against today's
transcript.

### 4.1 A compositional accept grammar, still closed and still whole-utterance

Keep the closed lexicon; stop requiring the utterance to *equal* one entry.
An utterance is an accept when it can be **completely segmented** into
accept phrases and a tiny set of connectives (`and`, `then`, `now`,
`please`, `just`), with at least one accept phrase and **no residue**. Same
for `LATER_PHRASES`. A segmentation that leaves any word over is not an
answer, exactly as now.

- *"go ahead and send it"* → `go ahead` · `and` · `send it` → accept.
- *"yes but change the time first"* → `yes` · residue → not an answer. The
  safety argument survives intact: it never rested on the list being short,
  only on the whole utterance being consumed.
- *"yes later"* → both kinds present → not an answer (see 4.2).

Lexicon additions that cost nothing: multi-word affirmations (*that's fine*,
*that's right*, *sounds good*, *looks good*, *go for it*, *I agree*,
*send it now*, *approve it*). Multi-word entries are covered by the span
gate — `react` re-asks any `Send` that `ours_coming_back` recognises — so
widening them cannot open the echo door. **One-word additions are the only
ones that need care**, because one word is immune to the span gate by
design (`MIN_SPAN_WORDS`): each must be checked against the tail of every
sentence the harness itself speaks, the way
`no_single_word_of_the_reask_is_an_answer` already checks the re-ask. `fine`
and `agreed` pass that test today; `right` and `correct` would need the
identity tail and the taint line re-read first.

`normalise` should also close the apostrophe gap: *"that's"* currently
becomes `that s`. Both the parser and the span gate use `normalise`, so the
change lands on both sides at once — which is the property the
`ours_coming_back` docstring insists on.

### 4.2 A non-answer carries the question over one turn, spoken again

When `react` returns `PassToModel`, remember the head id under the
`confirm_key` as *carried* — not armed. The words go to the model as now.
When that model turn finishes, `offer_for_turn` prepends the carried draft
if it is still pending and nothing has changed it: *"Still waiting on the
Lab Meeting draft. Say yes to send it, or later to leave it in your
outbox."* — a full re-ask through `say`, seeded into the echo window like
any offer, armed only if it reached the socket. Carried once; a second
`PassToModel` on the same draft says *"It's in your outbox"* and stops.

This is not the standing yes the module docs reject. The question is always
**asked again before a yes counts**; what changes is that a draft cannot
fall out of the conversation because the listener's first phrasing missed
the list. Today it would have turned *"I agree."* into one more exchange
rather than a dead end.

### 4.3 Tell the model the truth about the surface

Two edits, both cache-safe:

- The `VOICE_BLOCK` gains one sentence: *"If the user asks how to send or
  approve a draft, tell them to say yes after it is read back; never send
  them to a command or a page."* Static bytes, rides the prefix unchanged.
- The staged tool result gains a surface hint. `ToolCtx` grows an
  `Option<String>` the surface sets (`review_hint`), appended to the
  *"Drafted, not sent"* text in `agent.rs`: on the spoken door, *"The user
  will be asked aloud after this reply."* Per-call content, not prefix. The
  core stays surface-blind — it prints a string a caller handed it — and
  every other surface keeps today's wording.

Both would have replaced 19:37:45's *"You'll need to review and send it
through `mecha outbox`"* with the sentence the owner needed.

### 4.4 Supersession, so a re-drafted draft retires the one it replaces

Today's first draft is still pending because the model announced a withdrawal
it cannot perform. Two shapes, and the second is the structural one:

- A `withdraw` verb on the model's surface, restricted to items **this run
  staged** (`Provenance::session_id` and `call_id` make that checkable).
  Narrowing only — an injection that withdraws drafts has denied service,
  not exfiltrated — so it does not cross the *no lane promotes itself* rule,
  which is about acceptance. It needs a reason, and *"superseded"* is one.
- Or no verb: the outbox rejects an earlier pending item from the **same
  session, same tool** when a later one is staged, reason `superseded by
  <id>`, and the offer speaks only the survivor. This needs no model
  behaviour and cannot be gamed by prose; it is wrong only when a run
  genuinely means two drafts of the same kind — two invitations, two
  replies — which is the case `compose_offer`'s *"There is one more draft
  after that"* exists for. So: supersede only when the earlier item was
  **never offered** — staged by this session and not yet the head of any
  spoken question. That is today's case exactly: the first draft's turn was
  cut by the owner's barge-in at 19:36:40 before its offer could play, so
  it was never asked about, and the second draft replaced it in intent but
  not in the store.

The second is recommended; the verb is the follow-up if re-drafting an
*already offered* draft turns out to be common.

### 4.5 The offer is long, and its length is spent before the question

Forty-eight seconds is what the listener sat through. The readback itself is
right — *"the reviewable object is the thing itself"* — but three clauses
around it are debatable on a surface where every second is paid once:

- The **defaults clause** (*"all day false, calendar id primary"*) is pinned
  bookkeeping spoken on every calendar draft. #154 put it at the end and
  named it a default; the next step is to drop values that equal the
  schema default the listener has heard before, or to move it behind
  *"read it out"*.
- The **taint line** (*"I had read outside content when I wrote this, so
  listen to the addressing"*) fires on any session that has read a calendar
  or a mailbox — which is every session that drafts a calendar event. A
  warning that is always on is not a warning. `turn_taint` versus
  conversation taint is the distinction the interlock already makes;
  the taint line could speak only when the *drafting turn* read outside
  content.
- The **model's own preamble** is the biggest cost and is governed by the
  D10 block, which already says *"say one short clause and nothing more"*.
  Today's model said four sentences and asked a question. That is a rule
  for the learned-rules loop to enforce (a `voice`-domain correction), not
  a harness change.

### 4.6 No trailing question on a staging turn

One sentence in the `VOICE_BLOCK`: *"When you have staged something, do not
end with a question — the harness is about to ask one."* Closes 2.4.

### 4.7 Make the door observable

The confirmation path's own lines (`spoken answer was a span…`, `the reply
did not reach the socket…`, and a new one for *offer armed* / *answer
parsed as*) are `info`, and production runs `warn`. Either raise the three
that matter to `warn` — they are rare and each is a decision about a send —
or set `MECHA_LOG=info` on `mecha-serve.service`. Otherwise the next
incident is again reconstructed from the worker's TTS log.

## 5. Designing the half that does not exist

For the six-calls-a-corpus case — and for the day the owner says "delete
it" to a `mail_triage` offer on speakers — four shapes. The first is the
one the codebase already contains.

### A. Staging *is* the voice approver

Route a spoken turn's consequence-bearing calls through the outbox as a new
`OutboxKind::Action`, and let the fourth door — with §4 applied — offer and
release them exactly as it does a draft.

What it costs, in code: a third `OutboxKind` variant (a wire format; older
binaries load it as `Message`, as the `ARCHITECTURE.md` outbox section
already provides for); `speakable(Action) = true`; `mineable_as_writing`
excludes it (an edited `shell` command must never teach the writing miner a
voice rule); the page's `edit` stays allowed. `DraftView::of` already
renders an arbitrary argument object (*"Command: rm -rf build."*), and
`Surface::build` already executes any registered tool under the recorded
jail — `outbox approve` is tool-agnostic today. The route is per-run
(`begin_turn` constructs `OutboxRoute` per turn), so a spoken turn's route is
the configured one plus a voice list.

Which tools: **an explicit list, declared in config**, on the reasoning
`publish_tools` already carries — *a schema is the judged party's own
declaration and may never buy anything*. A capability-derived rule
(`destructive || external_send`) would stage `fs_write`, and the voice
workspace is where D9's *"Tuesday's sketch is an ordinary file on
Thursday"* lives; staging every note would make the safe surface the
useless one, which is the inverted read-only-trigger lesson D7 already
records. Suggested default: `mail__mail_triage`, `shell`, `kg_upsert`,
`kg_verdict`, `calendar_delete_event` (already routed). Owner's call.

What it costs, in behaviour: **the model does not get the result.** It is
told *"staged"* and moves on, so a spoken *"run the tests and tell me if
they pass"* becomes *"that's drafted"* — which is why the list must be
short and consequence-shaped, never "everything non-read-only". The
consequence asymmetry from `SPOKEN-OVERRIDE-DESIGN.md` §3 holds: a staged
action does nothing until released, and release is the tested path.

Why it composes with everything else: no barge-in conflict (it happens
between turns); survives a hang-up (the item is in the store; the page and
CLI can release it); no new echo surface (the same offer, the same gate);
and the interlock is untouched, because staging sends nothing and release
executes the recorded call under the recorded jail.

### B. Park and resume

The `questions.rs` shape applied to approvals: the approver parks the call
as an outbox `Action`, the run **ends** (`CancelReason::Parked`, partial
turn kept), the release runs the tool, and the session resumes with the
result as the next turn. This is A plus a resume, and it buys the one thing
A lacks — the model sees what happened. It is also a second mechanism where
A is a config list, and the resume runs a model turn the listener did not
ask for. Worth building only if A's *"the model moves on"* turns out to
matter in practice; it can be added on top of A without changing A's store
shape, because the item already records `session_id`, `call_id` and
`workspace`.

### C. Speak the question, close the stream, keep the run parked

The "true inline" shape: `WebApprover::ask` also emits a tap event; `pump`
speaks the question, ends the SSE stream **without** cancelling the run
(today a closed stream is treated as a hang-up and cancels), leaves the run
parked on the card, and `VoiceHost::speak` routes the next utterance to the
open card when it parses as an answer — instead of draining it. Approval
runs the tool in-run; the model gets the result; nothing is staged.

Its costs are the reasons not to start here: a new facade state (a session
that is *live but listening*), the `pump`/disconnect semantics inverted
for one case, a card whose timeout now spans the listener's silence, and —
the load-bearing one — **a spoken yes executes immediately**, with no
staged object to fall back on. The echo hazard `SPOKEN-OVERRIDE-DESIGN.md`
§2.1 records is unmitigated in exactly the band that matters: one word,
consequence now. A's whole safety story is that a wrong yes releases a
thing that was read aloud; C's wrong yes runs `shell`.

### D. A side channel through the worker (RTVI)

The worker already runs `RTVIProcessor` and speaks server messages to the
page; the question could go out-of-band and the answer come back the same
way. But the *answer* is still speech, still transcribed, still a
chat-completions request — so the worker would have to decide what is an
answer, which puts release policy in Python outside the harness. Reduced to
C with an extra protocol; not pursued.

## 6. Prior art, briefly

The field's answer to §2.1 is that yes/no is an **intent, not a string**:
Alexa's `Dialog.ConfirmIntent` is answered by `AMAZON.YesIntent` /
`AMAZON.NoIntent`, built-in intents whose utterance sets are Amazon's and
broad, matched by an NLU rather than by equality
([Using Yes/No Intents with Dialog Management](https://developer.amazon.com/en-US/blogs/alexa/post/2a70a910-7083-423a-bc12-ea1ae9f5e5a2/using-yes-no-intents-with-dialog-managemen),
[Dialog.ConfirmIntent](https://developer.amazon.com/en-US/docs/alexa/custom-skills/define-the-dialog-to-collect-and-confirm-required-information.html)).
mecha cannot use a model as that NLU — the decision must not enter a context
window — so §4.1's closed grammar is the deterministic equivalent: wide on
phrasing, closed on vocabulary. The voice-agent frameworks have no spoken
tool-confirmation pattern to borrow; LiveKit's HITL is about handing a call
to a person ([LiveKit HITL](https://livekit.com/blog/human-in-the-loop-voice-agents)),
and the Pipecat/LiveKit comparisons discuss context aggregation, not
approval ([framework survey](https://soniox.com/wiki/voice-agent-frameworks)).

## 7. Recommendation and sequencing

1. **§4.1–4.3, together, first.** They are the incident. Each is a small
   change in `review_policy.rs`, `voice/confirm.rs`, `voice/mod.rs` and one
   string in `agent.rs`; each has a test that fails on today's transcript
   (*"Go ahead and send it."* must release; *"Yes, but change the time
   first."* must not; a dropped draft must be re-offered once; the staged
   result on the spoken door must not say `mecha outbox`).
2. **§4.4 same-turn supersession and §4.6**, small and independent.
3. **§4.7**, so the next call is diagnosable from serve's own journal.
4. **§4.5** after a second real call has been listened to — the length
   changes are judgement calls and one call is one data point.
5. **§5.A** as its own arc, behind an explicit config list, once the
   fourth door has released a draft for real. It reuses every part §4
   hardens, and it should not be built on a door that has never worked.

The measurement `VOICE-RESEARCH.md` and `SPOKEN-OVERRIDE-DESIGN.md` §6 both
wait on — a real call staging a draft on purpose, to derive the playback
constant and re-measure `ECHO_SEGMENT_RMS` — happened today by accident and
was not instrumented. The next one should be.

## 8. Open, for the owner

- **Which one-word affirmations are worth their echo risk?** `fine` and
  `agreed` clear the tail test; `right`/`correct` do not without rewording
  the identity tail. Or no one-word additions at all.
- **§5.A's list.** Is `shell` in it? On the voice workspace a spoken *"run
  the script"* would then draft rather than run.
- **Supersession or a withdraw verb** for re-drafting a draft that *was*
  offered. Today's case — never offered — needs neither decision.
- **The taint line** — keep it always-on (honest, and long), or speak it
  only when the drafting turn itself read outside content.
