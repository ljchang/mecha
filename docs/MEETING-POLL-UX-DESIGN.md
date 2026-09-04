# The meeting poll as one conversation

*2026-09-04. Redesigns the group-poll half of SCHEDULING-DESIGN.md §5 from
the owner's chair. That document settled the mechanism (seeded candidates,
capability URLs, `rank_poll`/`clean_winner`); this one settles the
experience — what the owner types, what they review, and what happens
while they are not looking — and the lifecycle that has to exist for the
experience to be true.*

> **Status: ruled and built 2026-09-04 — phases 1–3 on
> `feat/meeting-poll-ux` (mecha) and `feat/meeting-poll-lifecycle`
> (mecha-factory); deploy owed.** §6 records the six rulings; §7 is the build
> order. Everything in §2 is measured against the tree at `mecha` 5e85ce5 and
> `mecha-factory` fdfbb7b, before the build. Two things the build settled
> beyond the text: a clean winner is **re-verified against live freebusy**
> before the event is made, and a collision flips the verdict to a pick with
> the reason (`conflict`); and the outbox now records a release's output
> (`OutboxItem::output`), so the reconciled `booked` for a pick carries the
> event id beside `via: outbox:<id>`. Deploy also needs
> `mail__calendar_create_event` in `[outbox] tools` — the documented default
> — or no pick card can be staged.

## 0. The one-sentence shape

> **Ask once, review once, then nothing until it is booked.** The owner
> says who, how long and what about; one card in the outbox shows the
> invitation they are about to send in their own name; and from the
> release onward a deterministic sweep — no model anywhere — mails the
> links, nudges the silent, books the clean winner as a native calendar
> invite, and brings the owner back exactly once, when there is a judgment
> only they can make.

Everything below serves that sentence. Where it conflicts with something
already built, the built thing changes.

## 1. The five moments

What the owner sees, in order. Each is a surface that exists today or a
named addition to one.

1. **Ask.** In chat, the TUI, Slack or voice: *"Find an hour for a lab
   meeting with Priya and Tal in the next two weeks."* The model resolves
   the names to addresses from what it already has (the graph, mail
   history, a roster CSV) and makes **one tool call** —
   `factory__poll_meeting_create` — with a title, participants, a duration,
   and optionally a window, a deadline and a message. Nothing else. It does
   not run a freebusy pipeline, write a file, or draft mail.
2. **Review.** One outbox card, kind `message`, whose reviewable object is
   **the invitation as the recipients will read it**: the owner's own
   sentence above a default block that explains the link, the deadline and
   what happens next. Beside it: who (name and address), how long, when
   answers close, the date window the times will be drawn from, and which
   account it sends from. `edit` changes the message or the template.
   Release does two things and the card says both: mint the poll on the
   box, and send each person their link from the owner's account.
3. **Wait.** The `/polls` monitor and the morning briefing show the
   lifecycle in one line — *"lab meeting · invites sent · 2 of 3 answered ·
   closes Fri 5pm"* — and the owner does nothing. The sweep nudges
   non-responders once, a day before the deadline.
4. **Decide.** Either there is nothing to decide — everyone answered and
   exactly one time is a plain *yes* for all — and the sweep books it: a
   calendar event from the owner's account with every participant as
   attendee, so each gets the provider's native invite, and the poll page
   closes with *"Booked: Tuesday 9 Sept, 2–3pm"* for anyone who revisits
   their link. Or there is judgment — a tie, a best time that costs someone
   an if-needed, someone silent at the deadline — and the owner gets **one
   more card**: a calendar-event draft for the top-ranked time, with the
   ranking and each candidate's reason in its description, and a one-key
   *pick* in the `/polls` monitor to swap in the second or third. Release
   books; reject closes the poll as *no time found*.
5. **Done.** The event exists, the invites are the provider's, the poll
   page states the outcome, and the record at home says what happened and
   when. The owner was interrupted at most twice: once to send, at most
   once to choose.

## 2. What exists, and what stops each moment today

Measured, not remembered. The MCP tool, the CLI verb and the box's poll
pages all work; the *sequence* does not, because the pieces were built to
the mechanism and the seams were left to the model.

| # | Gap | Where | Which moment it breaks |
|---|---|---|---|
| G1 | **Invitations are never sent.** `describe_created` returns the URLs and expects the model to draft N mails, one per link. `~/.mecha/factory/polls/` does not exist on the owner's machine: no meeting poll has ever completed this path | `mecha-factory-publish/src/mcp.rs` | 2 |
| G2 | **A staged create fails at release.** `factory__poll_meeting_create` is outbox-routed (correctly — it mints a public page), so the tool executes when the owner releases it; `create_meeting` then refuses a `freebusy` file older than an hour. A poll staged at five and released the next morning cannot be created | `polls.rs::create_meeting`, the owner's `[outbox] tools` | 2 |
| G3 | **The tool takes files the model must manufacture.** `policy` and `freebusy` are paths; the model has to run `mecha-mail freebusy`, write the output to the workspace, know where the policy lives, and pass both. The CLI takes freebusy on stdin, the MCP tool takes a path, and neither defaults anything | `mcp.rs` `poll_meeting_create` schema | 1 |
| G4 | **No lifecycle.** SCHEDULING-DESIGN §5.4's nudge, close, finalize and auto-book sweep are unbuilt; `polls status --json` hands the verdict to the agent, which is the model doing by hand what §2.2 says cron is for. Nothing books, nothing nudges, nothing closes | — | 3, 4, 5 |
| G5 | **No message.** Nowhere to say *why* — the owner's sentence — and no default instructions for the recipient | `mcp.rs` schema | 2 |
| G6 | **No holds.** §5.5's rule that an open poll's candidates enter the booking engine's `holds[]` is unbuilt: the public booking page can sell a slot the poll is still asking about | `slots push` | 4 |
| G7 | **Wrong review kind.** The owner's config lists `poll_meeting_create` under `publish_tools`, so the card is `Publish`-kind: `edit` is refused and the reviewable object is "the rendered page" — but the thing being approved is a letter in the owner's name | `~/.mecha/config.toml`, `website/docs/factory/onboarding.md` | 2 |

Two of these (G2, G7) are a day's confusion each; G1 and G4 are the
feature. G3 and G5 are what made the first attempt feel like plumbing.

## 3. The design

### 3.1 One call, three required inputs

`factory__poll_meeting_create` keeps its name and its outbox route and loses
most of its schema. Required: `title`, `participants` (or `roster`),
`duration_minutes`. Optional, each with a stated default:

| input | default | note |
|---|---|---|
| `message` | none | the owner's sentence, rendered above the default block (§3.3) |
| `deadline` | 3 days after release, 5pm in the policy's zone | RFC 3339 or a date; see §5 for the clamp |
| `earliest` / `latest` | deadline + `min_notice_hours` … policy horizon | a window in dates; "next two weeks" becomes `latest` |
| `account` | the mail default | which mailbox the invitations and the event come from; pinned into the card like any `mail_send` default |
| `instrument` | the one booking instrument on record | an error naming the choices when there are several |
| `poll_id` | `<slug(title)>-<yyyymmdd>` | must still be new |
| `max_candidates` | 10 | 5–15 is the point |
| `policy` | the instrument's own `[availability]` file | `~/.mecha/instruments/<instrument>-policy.toml`; a model-supplied path is still jailed |
| `freebusy` | **the slots pipeline's last answer** | see below |

**The freebusy comes from the pipeline that already runs.** The
`mecha-slots` timer pipes `mecha-mail freebusy --days 60 --json` into
`factory-publish slots push` every two minutes to keep the booking page
fresh. `slots push` will write its stdin to
`~/.mecha/instruments/<instrument>.freebusy.json` (temp-sibling-and-rename)
as a side effect: *the last busy time the pipeline saw*. `create_meeting`
reads that when no `freebusy` is supplied, and the freshness refusal
stays exactly as it is — an hour — but is now a statement about the timer,
and the refusal says so: *"the slots pipeline has not run since 14:02; check
`mecha-slots.timer`"*. No new binary pairing, no model transcribing
intervals, and the file is as fresh at release as at staging, which is what
closes G2. The path is fixed, not model-supplied, so it needs no jail — the
same footing as `~/.mecha/factory/publish.key`.

**Candidates are computed at release, and the card says so.** A poll
staged at five and released at nine is seeded from nine o'clock's calendar.
Staging executes nothing, so the card cannot count them; it shows the
window they will be drawn from and says "times drawn from live
availability at release", and the release output and the record carry the
list. This is §2.1's own reasoning applied: the candidate set is
deterministic code over the owner's own calendar, which the slot push
already declines to review.

### 3.2 The record is the state machine

`~/.mecha/factory/polls/<poll_id>.json` already holds what the box never
learns — names against addresses and URLs. It grows a `lifecycle` block,
and every actor below is a consumer of that one file, exactly as
`~/.mecha/requests/*.json` is the seam between `factory-publish drain` and
`mecha-mail bookings`:

```jsonc
"lifecycle": {
  "account": "dartmouth",
  "message": "Can we find an hour before the grant deadline?",
  "deadline": "2026-09-08T21:00:00Z",
  "invites":  { "Priya": "2026-09-05T13:02:11Z", "Tal": null },   // sent_at per name
  "nudged_at": null,
  "closed_at": null,
  "verdict":  null,            // "book" | "pick" | "no_time", set by the sweep at close
  "book":     null,            // {start, end} the clean winner, or the owner's pick
  "pick_item": null,           // outbox id of the judgment draft, when verdict == "pick"
  "booked":   null,            // {event_id, at} once the event exists
  "resolution": null           // the sentence the poll page shows
}
```

Closed enums written to an append-only store are wire formats: unknown
values load as `null`, never fail the record.

### 3.3 The invitation

Templated at home, sent from the owner's account, one message per
participant carrying that person's link. The subject and body templates
are **`default`s declared in the tool's own schema**, so staging pins them
into the card (`tool::with_schema_defaults` — the same mechanism that pins
`mail_send`'s account) and the owner edits them there; the record stores
what was released and the sweep substitutes `{title}`, `{duration}`,
`{deadline_local}`, `{url}` and `{message}` per participant. No separate
template file: the card *is* the template, per poll, and an edit there is
the owner's voice in the owner's outbox.

```
Subject: When can you meet? — {title}

{message}

I'm finding a time for "{title}" ({duration} minutes). Could you mark
which of these times work for you?

    {url}

Please answer by {deadline_local}. Tap a time to cycle through yes,
if needed, and no. The link is yours alone — it is how the page knows
the answers are yours — so please don't forward it. Once everyone has
answered, I'll send a calendar invitation for the time that works.
```

The card shows `message` and the template as prose with the recipient
list beside them. The writing miner may learn from an edit — it is the
owner correcting a letter in their own voice, which is the outbox's whole
learning signal — so the card is `Message`-kind, and `poll_create` /
`poll_meeting_create` leave `publish_tools` (G7). The onboarding page's
routing block changes with it.

### 3.4 The sweep — three verbs on the timer that already ticks

`mecha-slots.service` runs `factory-publish drain; mecha-mail bookings`
every two minutes. The poll lifecycle is three more idempotent verbs on the
same line, each reading the record and writing only its own fields:

```
factory-publish polls sweep; mecha-mail polls --account dartmouth; mecha polls sweep
```

- **`factory-publish polls sweep`** — the box-facing half, no calendar.
  For every record with no `closed_at`: fetch status; if everyone has
  answered or the deadline has passed, rank, set `verdict` (`book` with the
  clean winner, `pick` with the ranked top three and a reason per row,
  or `no_time` when nothing is feasible for anyone), and set `closed_at`.
  When `booked` or `resolution` has appeared since, close the poll on the
  box with that resolution — the deadline already freezes answers there,
  so closing can wait for the outcome sentence. Also marks `nudge_due`
  when the deadline is 24h out and someone is silent.
- **`mecha-mail polls --account <a>`** — the mail-and-calendar half.
  Sends invitations still `null` in `invites` and the nudge when due, from
  the templates in §3.3; creates the event for a `book` (title, the slot,
  every participant as attendee, the ranking summary in the description),
  writes `booked`, appends to `~/.mecha/mail/polls.jsonl` — the ledger is
  the idempotency, as it is for bookings. Owns no decision: it does what
  the record says is due.
- **`mecha polls sweep`** — the owner-facing half, because only `mecha`
  holds the outbox. For a `pick` verdict with no `pick_item`, stage a
  `mail__calendar_create_event` draft for the top candidate (the normal
  registry, the normal route, the normal release) and record its id.
  When that item is `sent`, read the slot from its released `args` and
  the event id from its recorded output, and write `book`, `booked` and
  `resolution`. When it is `rejected`, write `resolution` as
  *"No time found"* (or the reject reason) and `verdict: no_time`. Nothing
  else — it is the same thin glue the TUI's `/polls` modal already is.

Nothing here runs a model, and nothing sends to a stranger without either
a template the owner shipped or a release the owner clicked (SCHEDULING
§6, unchanged). A tick that finds the box unreachable leaves the record
untouched and says so in the journal; the next tick retries.

### 3.5 The pick

The judgment card is a real `calendar_create_event` draft, so releasing it
is booking it, and the reviewer sees the same object they would see for
any calendar draft. What makes it a *pick* rather than an edit:

- Its description carries the ranking — *"1. Tue 9 Sept 2pm — everyone
  can. 2. Thu 11 Sept 10am — Tal if needed. 3. Fri 12 Sept 3pm — Priya
  hasn't answered."* — and the summary line names the poll.
- The `/polls` monitor gains one key: **`p`** cycles the draft's slot
  through the ranked candidates (`OutboxStore::update_args` on
  `start_time`/`end_time`, nothing else), and the row shows which is
  loaded. `mecha polls pick <poll> <n>` is the same edit from the CLI.
- Release books the loaded slot; the sweep closes the poll with the
  sentence. Reject closes it as *no time found* — the participants are
  not mailed, because there is nothing templated to say and the owner is
  right there to say it.

### 3.6 Holds

`slots push` reads the open poll records beside the policy and passes
their candidates as `holds[]` to the engine it already calls. One
argument that was `&[]`; §5.5 built. The booking page cannot sell a slot a
poll is asking about, and a closed poll's candidates fall out on the next
tick.

## 4. What the owner is told, and where

| surface | shows |
|---|---|
| the outbox card (moment 2) | the message and the invitation template as prose, the recipient list, duration, deadline, the date window, the sending account |
| `/polls` monitor, briefing, `mecha doctor` | one lifecycle line per open poll: *invites sent 3/3 · 2 answered · closes Fri 5pm*; *needs a pick* as a finding, never a silent queue |
| the pick card (moment 4) | the event draft, the ranking with reasons, which slot is loaded |
| the poll page | the outcome sentence, for anyone holding a link |
| `mecha polls status <id>` | the record and the box's tally, verbatim |

## 5. The decision policy, stated

The user's question was *when do we make the call, and how do we choose?*
The ruling on 2026-08-07 was **auto with guardrails**; this states the
guardrails as numbers and adds the one knob a person who trusts the ranking
more than the owner does would want.

**When the poll closes.** The earlier of: every participant has answered;
the deadline passes. Default deadline is **three days after the
invitations go out, at 5pm in the policy's zone**. Candidates are chosen to
start no earlier than `deadline + min_notice_hours`, so a poll never closes
after its own first option; an owner-supplied deadline that would violate
that is clamped and the card says so.

**The nudge.** Once, to non-responders only, 24 hours before the deadline;
skipped entirely when the deadline is under 36 hours away at send, because
a nudge eleven hours after an invitation is nagging.

**The verdict.** `rank_poll`'s order, unchanged: feasible first, then most
*yes*, then fewest *if needed*, then earliest. Then:

| everyone answered? | best slot | default (`auto_book = "unanimous"`) | `"feasible"` | `"never"` |
|---|---|---|---|---|
| yes | exactly one unanimous *yes* | **book** | book | pick |
| yes | two or more unanimous | pick | book the earliest | pick |
| yes | best needs someone's *if needed* | pick | book | pick |
| yes | nothing feasible | pick, titled *no time works for everyone* | same | same |
| no (deadline) | any | pick, silent people named | same | same |

The numbers and the knob live in the policy file's `[poll]` table, every
key optional:

```toml
[poll]
auto_book = "unanimous"     # unanimous | feasible | manual
deadline_days = 3           # answers close this many days after the invitations
deadline_hour = 17          # at this hour, in the policy's zone
nudge_hours_before = 24     # one nudge to the silent; 0 disables it
nudge_min_lead_hours = 36   # no nudge when the deadline was closer than this at send
```

A per-call `deadline` overrides the two deadline keys for that poll. The
default is the existing ruling; `"feasible"` is for the owner who would
rather never see a card than protect a colleague's if-needed; `"manual"`
is for the owner who wants to look every time. There is no fourth value
that books over a silent participant — a meeting someone never agreed to
is the failure the poll exists to prevent.

**What booking means.** One event on the owner's calendar in the named
account, every participant as attendee, notifications on — so the
confirmation *is* the provider's invite, RSVPs flow back, and a later
cancellation is a native retraction (the 2026-08-08 mail inversion,
unchanged). The silent participant at a picked time receives the invite
too; that is how they learn.

**What the model never does.** Choose a time, close a poll, or send to a
participant. It makes one call at the start, and it can *read* the
lifecycle (`poll_status` returns the record's lifecycle beside the tally,
so "how's the lab-meeting poll?" has an answer), but every transition is
the sweep's or the owner's.

## 6. Rulings — 2026-09-04

Recorded so they are not re-litigated.

1. **`auto_book` defaults to `unanimous`** (the 2026-08-07 ruling), with
   `feasible` and **`manual`** as the knob — the owner asked for the
   manual value by name.
2. **Deadline three days out at 5pm local, one nudge at −24h — and every
   number configurable**, in `[poll]` (§5). SCHEDULING §8.4 is closed by
   this.
3. **Candidates are recomputed at release.** The card shows the window,
   not a count — staging executes nothing.
4. **The pick is an outbox calendar draft**, with the `p` key in `/polls`
   to swap candidates. Not the questions store: one review surface, and
   the object released is the booking itself.
5. **Reject on the pick card closes the poll as "no time found", no mail.**
6. **`poll_create` / `poll_meeting_create` are `Message`-kind** and leave
   `publish_tools`. The kind governs review, not routing: a `Publish` card
   for a poll leads with local paths that do not exist, refuses `edit` on
   the owner's own sentence, and hides the outbox's one positive signal
   (an invitation sent as drafted) from the writing miner. A third
   `OutboxKind` was considered and refused — a closed enum in an
   append-only store is a wire format, and every reader would change to
   buy nothing `Message` lacks.

## 7. Build order

Three repositories, each phase landable and testable alone; the first
phase alone makes the tool usable by hand.

1. **`mecha-factory` — the tool and the record.** `slots push` tees the
   freebusy cache; `create_meeting` defaults `freebusy`, `policy`,
   `instrument`, `poll_id`, `deadline`, `account`; `message`, `earliest`,
   `latest` arrive; the record gains `lifecycle` and the candidate list;
   the subject and invitation templates are schema defaults; `polls sweep`
   (the box half, §3.4);
   `poll_status` returns the lifecycle; holds in `slots push` (§3.6). Tests:
   the freshness refusal names the timer; a poll created with no
   `freebusy` reads the cache; the sweep's verdict table (§5) as a
   property test over `rank_poll`; a record with an unknown verdict loads.
2. **`mecha-mail` — `polls` verb.** Substitutes the record's templates;
   invitations, nudge, the booking, the ledger. Tests mirror
   `bookings.rs`: every rule against one clock, each fires once, never
   after the fact.
3. **`mecha` — the owner's half.** `mecha polls sweep` and `pick`; the
   `/polls` monitor's lifecycle line and `p` key; the briefing and doctor
   findings; the timer line; `publish_tools` and the onboarding docs;
   `website/docs/factory/polls.md`'s times-poll section rewritten around
   the five moments. Tests: the pick edits only the two time fields; a
   sent pick item closes the record; a rejected one resolves it.
4. **Later, and not in this arc:** subtracting *participants'* freebusy
   where readable (same-tenant colleagues — SCHEDULING §5.1's silent
   subtraction); a web page for the monitor; `polls extend` to reopen a
   `no_time` poll with a wider window; a `link`-audience times poll.

## 8. Left out on purpose

- **An open-link meeting poll.** The first attempt reached for one because
  sending per-person links was manual. With sending automatic, the
  per-person link is invisible to the owner and is what makes *"everyone
  answered"* a decidable question; an open link has no *everyone*. The
  roster CSV covers a large group. If a real case arrives, it is a
  `times` question on a `link` audience with `auto_book = "manual"`, and it
  is a day's work then.
- **Rescheduling a booked meeting.** The event is the provider's; move it
  there and the attendees get the native update. A poll is for finding a
  time, not for owning one.
- **A model in the sweep.** Every transition above is a table, a
  transaction or a template. The one place a model appears is moment 1, and
  it makes one call.
- **Per-participant reminders about the meeting itself.** The provider's
  invite carries the attendee's own reminders; the booking page's reminder
  tiers exist for visitors who have no event of their own.
