# Scheduling — the booking page and the seeded group poll

*2026-08-07. Expands §9 of PUBLIC-SURFACE-DESIGN.md into a buildable design.
The four user decisions recorded here: week-column layout, seeded week grid
for the group flow, auto-finalize with guardrails, booking page ships first.*

> **Status 2026-08-08: built and deployed; a front-end pass awaits deploy.**
> Steps 1–8 and the poll are code-complete, tested in both repos, and live —
> the booking lifecycle end to end (page, claim, provider-native invite from
> the user's account, manage/cancel) and the group poll (seeded candidates,
> capability URLs, tri-state answers, `rank_poll`/`clean_winner`). §4 was
> redesigned along the way (provider-native invites; SES is account plumbing
> only). The same evening a front-end pass shipped (factory `1d531a8`,
> deployed): §3.2's split-step reveal and duration switch, §5.3's
> tap-to-cycle/drag-paint/heat layer, a live `slots.json` the open page
> polls, and the POST error path. Left out
> of §5.3 on purpose: the axis-locked touch gesture and a separate Group
> tab — inline heat carries most of the value. Still open: the deterministic
> auto-book sweep for the clean-winner case (today `polls status --json`
> hands the verdict to the agent, which books and closes).

This is the first official mecha-factory artifact: a replacement for
youcanbookme that grows when2meet's group half — except the group half is
seeded, so participants never see a blank grid, only the slots the user can
actually do. It is also the first instrument that needs **state on the box**:
today the factory is forms-in, bytes-out, and a booking page needs a slot to
be claimable exactly once, mail to fire at 24h and 1h before a meeting, and a
cancel link that works at 2am with the home laptop closed.

The one-sentence shape, and everything below serves it:

> **Home computes meaning; the box collects and repeats.** Availability is
> computed at home by deterministic code and *pushed* to the box as data. The
> box serves it, claims slots atomically, and sends only mail that was
> templated in advance. No model runs on the public surface, and nothing a
> stranger sees synchronously requires home to be reachable (§14.4).

---

## 1. The three layers

| Layer | Owns | New work |
|---|---|---|
| `mecha-mail` | the calendars | `calendar_freebusy` (tool + CLI) |
| mecha, at home | meaning: what "available" means, what gets booked | availability engine, slot-refresh pipeline, drain handlers |
| `mecha-factory`, the box | collection: serving, claiming, templated mail | booking instrument, claim tables, mail jobs, manage links |

### 1.1 `calendar_freebusy`

Google `freeBusy.query`, Graph `getSchedule` (20 mailboxes, 62 days,
chunked), fan-out across accounts, **intervals only** — never
`findMeetingTimes`, because ranking is our engine's job and a provider's
opinion of a good time is not portable across a mixed-provider group. Two
front ends over one library function: the MCP tool (so the model can answer
"when am I free Thursday") and a **CLI verb** (`mecha-mail freebusy`), which
is what the refresh pipeline uses — see §2.2 for why the pipeline must not
contain a model.

### 1.2 The availability engine

`availability(windows, busy, holds, bookings, now) -> Vec<Slot>` — pure,
deterministic, unit-tested to death, living beside the other pure code. Inputs:

- **Windows**: weekly recurring bookable hours in the user's IANA zone, plus
  date overrides (block a day, add an unusual one).
- **Policy**: durations `[30, 60]`, buffer before/after, minimum notice,
  maximum horizon, per-day cap on bookings.
- **Busy**: the freebusy intervals, merged across accounts.
- **Holds**: slots offered to an open group poll (§5.5) and soft holds — both
  subtract, which is Calendly's hold-slots behaviour and the reason the
  engine takes them as input rather than learning about polls.
- **Bookings**: already-confirmed meetings, so the per-day cap counts them.

DST falls out of doing all arithmetic in the IANA zone and emitting UTC
instants — the same both-directions rule cron.rs already earned.

### 1.3 What the box never does

The box **subtracts, never adds**: it serves `pushed slots − its own live
holds − its own confirmed bookings`. It cannot widen availability, because it
never computes any — a compromised box can hide slots (a nuisance) but cannot
offer a slot the user cannot do (a real harm). Same direction-of-trust as
"the server can only return objects that validate against a schema mecha
uploaded."

---

## 2. Freshness — the slot push

### 2.1 The endpoint

`PUT /v1/instruments/<id>/slots`, authenticated with the publish key. Body is
shape-validated and capped: `{generated_at, horizon_days, slots: [{start,
end, duration}]}`, RFC 3339 UTC, a few KB. Replaces the cache atomically
(one SQLite transaction). This is **data, not a publish** — no version is
minted, no alias moves, no outbox review. A review per refresh would either
kill the freshness or teach the reviewer to click yes, and the push carries
nothing a human needs to judge: it is the output of deterministic code over
the user's own calendar, already public in exactly this granularity by the
page's existence.

### 2.2 The pipeline is a command, not a prompt

```
mecha-mail freebusy --days 60 --json | mecha-sched slots | factory-publish slots push book
```

on a systemd timer, every 15 minutes. Deliberately **not** a mecha trigger:
triggers run prompts, and a model transcribing intervals is a model that can
mistranscribe them — this is `scripts/ruminate.sh`'s rule ("scheduled
commands are what cron is for") applied to data flow. Zero tokens, works
unattended, and a stale push degrades exactly the way youcanbookme's sync lag
does: the page shows slightly old slots and the claim step (§3.3) catches
the difference.

Staleness is bounded from both ends: the box stamps the cache with
`generated_at` and the page states it; a cache older than a configurable
limit (default 24h) makes the page say so rather than sell yesterday's
Tuesday.

---

## 3. The booking instrument

### 3.1 One manifest, two readers

A new manifest kind: `kind = "booking"` beside the request types. One TOML,
authoritative at home, pushed to the box like any type — but the two ends
read different halves:

```toml
id = "book"
kind = "booking"
version = 1
title = "Book a meeting with Luke"

[availability]            # home reads this; the box ignores it entirely
timezone = "America/New_York"
durations = [30, 60]
buffer_minutes = 10
min_notice_hours = 24
horizon_days = 60
per_day_cap = 3
[[availability.windows]]
day = "tue"
start = "13:00"
end = "17:00"
# ...

[policy]                  # the box enforces this
cancel_cutoff_hours = 2
hold_minutes = 30
reminders = ["24h", "1h"]

[[fields]]                # the details form — existing FieldKinds, unchanged
name = "requester_name"
kind = "text"
max_length = 120
required = true
# requester_email (email, the verification field), topic (long_text), ...

[mail]                    # every message the box will ever send, templated
confirmation_subject = "Confirmed: {title} on {when_local}"
# ... reminder / cancelled / rebook bodies, {placeholder}-interpolated
```

The box never computes from `[availability]`; home never enforces
`[policy]`. Free-text-ness of the detail fields stays derived from kind, and
drained booking records carry `free_text` exactly as request records do — a
booking's "what do you want to discuss" goes through the same quarantine as
any stranger prose (§6.1).

### 3.2 The page

New gate routes, beside `/f/`:

```
GET  /s/<handle>/<id>              the weekly page (server-rendered)
POST /s/<handle>/<id>              slot + details → soft hold + magic link
GET  /s/<handle>/<id>/c/<token>    the claim: hold → confirmed
GET  /s/<handle>/<id>/m/<token>    the manage page
POST /s/<handle>/<id>/m/<token>    cancel
```

**Week columns** (the user's pick): seven day-columns, each a vertical list
of slot buttons, prev/next-week paging, a `30 min | 60 min` segmented
control, and a timezone selector under the grid showing a friendly name plus
the live local time ("Eastern Time — 4:12pm"), never a UTC offset.

Rendered server-side from manifest + slot cache, styled by the existing
theme tokens (`nocturne`/`paper`), zero framework — the gate's CSP
(`script-src 'self'`, `form-action 'self'`) already permits exactly this
shape. Progressive enhancement, in the form renderer's tradition:

- **JS off**: slots are radio inputs, times rendered in the host's zone and
  labelled as such, POST works, everything books. The safe direction is the
  working direction.
- **JS on** (`booking.js`, external file like `form.js`): times re-render in
  the visitor's `Intl`-detected zone from `data-utc` attributes; picking a
  slot uses Calendly's split-button confirm (the button divides into
  time + "Next" — no modal, no mis-taps); the details form swaps in with the
  chosen time kept visible; week paging fetches nothing (the cache horizon
  ships with the page).

Mobile: the column row becomes a horizontally paged day view with 44px+
targets — a pager, not a shrunken grid. Accessibility: the slot list is
plain `<button>`s (a 1-D list earns no grid semantics); full ARIA grid
treatment is reserved for the poll grid (§5.3).

### 3.3 The claim — two phases, both atomic

1. **Submit → soft hold.** The POST validates (same `validate_at` path as
   forms), then inserts a hold on the slot with TTL `hold_minutes` in the
   same transaction that checks no confirmed booking or live hold covers it.
   The visitor is told: *held for 30 minutes — click the link in your email
   to confirm.* Losing the race renders the refreshed slot list with "that
   time was just taken", never an error page.
2. **Magic-link click → confirmed.** The existing verification machinery,
   with the claim folded into the same transaction: hold still valid, slot
   still in `cache − holds − bookings`, flip to `confirmed`, mint the manage
   token, enqueue the mail jobs, mark `queued` for drain. A hold that
   expires un-clicked simply lapses; the slot returns on the next page load.

Why hold at submit rather than claim at submit: the email gate is minutes
long, and an unverified claim is a free denial-of-service on any slot — type
a stranger's address, never click. The hold costs its holder nothing to
lose and an attacker 30 minutes × a per-email/per-IP cap of live holds
(default 2) to abuse. Unverified never enters the queue, unchanged.

Booking states, box-side:

```
held ──(link click)──▶ confirmed ──▶ completed        (end time passes)
  │                        │
  └──(TTL lapse)  ✕        ├──▶ cancelled_by_booker   (manage link)
                           └──▶ cancelled_by_host     (drained from home)
```

### 3.4 Home's half of a booking

Confirmed and cancelled records drain like any request. The handler —
deterministic code on the drain path, not a model — creates the real
calendar event (`calendar_create_event`, **without attendee emailing**:
`sendUpdates=none` / Graph equivalent) and keeps the `booking_id → event_id`
mapping in its own store; a drained cancellation deletes the event. The
attendee's calendar copy comes from the ICS the box already mailed (§4.2),
so nobody gets two invitations — the provider event exists to block the
user's own freebusy and to put the meeting in front of their eyes.

Home being unreachable degrades exactly as §14.4 promises: bookings still
confirm, mail still sends, the queue grows. What lags is the user's own
calendar copy — and the box's local subtraction (§1.3) keeps the slot from
double-selling meanwhile.

---

## 4. Mail — the invite is the provider's, the plumbing is SES's

*Redesigned 2026-08-08, at the user's direction: booking mail comes from the
user's own account; SES is account plumbing only.*

The split follows who the mail is from. A magic link or a signup invite is
the **box** talking — SES, templated, part of serving strangers alone
(§14.4). But a booking confirmation is **the user** talking: the visitor
booked a meeting with a person, and mail from that person's own mailbox is
what they expect, what threads their replies correctly, and what delivers
best.

And once the sender is the user's account, the confirmation should not be a
hand-rolled ICS at all — it is the **provider's native invite**. Home
creates the calendar event *with the requester as attendee*
(`sendUpdates=all`; Graph mails attendees on create unconditionally, which
was the reason to avoid attendees before and is the feature now). What that
buys, all at once: the most deliverable invite that can exist, working
Accept/Decline whose RSVP flows back to the user's event, UID and SEQUENCE
as the provider's bookkeeping, and cancellation as `delete` with
notifications on — a native retraction from the visitor's calendar. The
entire hand-assembled METHOD:REQUEST module, the SES raw-MIME work, and the
deliverability test matrix are deleted, not deferred.

### 4.1 The manage link

Cancellation must work without an account, so the capability URL survives:
`GET /s/<handle>/<id>/m/<token>` renders state, POST cancels — GET-safe
because scanners prefetch, token hashed at rest on the box, all states
answered honestly (active / inside the cutoff / already cancelled / past /
dead link). The token is minted at confirm, box-side — so the box writes
the full manage URL into the queue payload (`_manage_url`) in the same
transaction, and it drains home like the rest of the machinery keys. Home
puts it in the event description, which both Gmail and Outlook render in
the invite. Plaintext transits the queue briefly; acceptable for a
capability whose job is to travel in email.

A visitor's cancel queues a cancellation record; home's `bookings` verb
deletes the event through the ledger's `booking_id → event_id` join, and
the provider mails the retraction.

### 4.2 What rides home's timer, and what §14.4 still guarantees

Everything a stranger sees **synchronously** still comes from the box alone:
the page, the hold, the confirmation screen. What moved to home's drain
cadence is the *email* leg — the invite arrives minutes later, not
instantly, and queues while home is down. The deployment this serves runs
home on an always-on machine; the degradation is latency, stated plainly on
the confirmation page ("a calendar invite is on its way").

Reminders (24h/1h) become a later, optional increment: deterministic
templated sends from the user's own account on the same timer family as the
slot refresh — no model, no outbox, the same class of machinery as the
provider invite. The attendee's own calendar reminders cover much of the
need natively.

## 5. The group poll

### 5.1 Creation is private and staged

The user asks in chat/TUI: participants, duration, a window, a deadline.
The engine computes *the user's* candidates; for any participant whose
freebusy is readable (`calendar_freebusy` across configured accounts), their
busy time is subtracted silently. What survives — typically 5–15 slots, by
construction all feasible for the user — becomes the poll. **Creating the
poll is the model acting on third parties** (each participant gets an
email), so it stages through the outbox as one reviewable item: candidate
slots, participant list, invite text. One release covers the set; the box
then sends per-participant invites from the template.

If subtraction leaves one obvious slot for everyone, steps below collapse:
the tier-1 and tier-3 cases are the same code path (§9.2's point), and the
poll is skipped in favour of a staged direct invitation.

### 5.2 Participants get capability URLs

`GET /p/<handle>/<poll>/<token>` — per-recipient, hashed at rest, no
account, no password, no name typing (the token *is* the identity, which
kills when2meet's typo'd-duplicate-respondent failure). The page shows the
**same week-column frame as the booking page** with only candidate cells
live — never a blank 7×24 grid.

### 5.3 The grid

Tap a cell to cycle **yes → if-needed → no**; drag paints (pointer events,
`setPointerCapture`, `elementFromPoint` per move; the anchor cell's state
decides the whole stroke's mode, when2meet's exact mechanic, so a mixed
drag never flickers). Mobile: `touch-action` locked so vertical drag paints
within a day and horizontal swipe pages days — the axis is decided in the
first few pixels. Every commit POSTs immediately; there is no save button.
JS off: each candidate renders as a yes/if-needed/no radio row plus a submit
button — the Rallly shape as the degraded mode.

A "Group" tab shows the heatmap: discrete steps (`available / respondents`,
≤6 distinct shades in OKLCH on the theme's accent), full-agreement cells
specially marked, tap-for-names on every cell, counts in each cell's
accessible name so the information survives color-blindness and screen
readers (never color alone). The paint grid takes the full ARIA treatment —
`role="grid"`, roving tabindex (one tab stop), arrows move, Space toggles,
Shift+arrows extend, mutations announced via a polite live region. No
mainstream calendar widget has this; it is cheap here and genuinely
differentiating.

### 5.4 Nudges, close, finalize

Non-responders get one templated nudge before the deadline (`mail_job`,
same sweep). The poll closes when everyone has answered or the deadline
passes; the closed poll drains home, and **deterministic code** ranks
feasibility, cost and fairness separately (per CalBench — a scheduler that
always finds a time by quietly costing someone their afternoon scores 100%
on the only number most systems report).

**Finalize, auto with guardrails** (the user's pick):

- **Clean close** — a unique slot every respondent marked plain *yes*:
  book it. The handler creates the event, the box sends everyone the
  confirmation + ICS, done. No human in the loop, which is the promise the
  poll page makes.
- **Anything else** — a tie, a best-slot that needs someone's if-needed, a
  non-responder at deadline: the top three are staged through the outbox
  with reasons attached ("Tuesday 2pm, but Tal moves something"), and the
  user's release is the decision. Judgment where there is judgment,
  automation where there is none.

### 5.5 Holds

The poll's candidate slots enter the engine's `holds[]` the moment the poll
is released, so the single-booking page cannot sell a slot out from under
an open poll; close or expiry releases them. This is one field of data, not
a coupling — the engine already takes holds, and the booking page never
learns polls exist.

---

## 6. Security posture — what stays true

- **No model on the box**, still. Every new behaviour is a table, a
  transaction, or a template.
- **Free text is quarantined, still.** Booking topics and poll comments
  drain as `free_text` and reach a privileged run only through the
  extraction pass. The typed fields — slot, duration, tri-state answers —
  are enums and instants the origin validated; nothing a stranger types
  changes what kind of thing their submission is.
- **The claim sits ahead of everything**, like the interlock: it is a
  property of the transaction, not of anyone's judgment, and two visitors
  cannot both hold a slot no matter what any page says.
- **Capability tokens are hashed at rest** (the box-is-lost assumption:
  its disk yields verifiers, not credentials), scoped to one object and
  one capability, rotated on change, expiring at natural ends.
- **Abuse economics**: holds cost the attacker a capped concurrency and
  cost the user nothing permanent; sends stay under the existing
  per-recipient and per-user budgets; slot data was public by intent.
- **The model still cannot**: move a slot, mark a poll closed, decide
  consent, or cause mail to a stranger without either a template the user
  shipped or an outbox release the user clicked.

New surface, named: the box now holds *appointments* — who is meeting the
user and when. That is real data about third parties, owner-only like the
request store, covered by `retain_days`, and worth remembering when writing
`factory user` docs: a tenant's bookings are exactly as private as their
request queue.

---

## 7. Build order

Booking page first (the user's pick, and §12 step 8's own ordering); the
poll lands on proven machinery.

1. **`calendar_freebusy`** — library + MCP tool + CLI verb. Useful the day
   it lands, testable against real calendars immediately.
2. **The availability engine** — pure, with the policy vocabulary from
   §3.1. Property-tested around DST, buffers, caps.
3. **Slot push** — `factory-publish slots push`, the endpoint, the cache
   table, the systemd timer unit. `mecha-sched` glue binary.
4. **The instrument** — manifest `kind = "booking"`, `check()` rules,
   gallery example (the exhaustive `kind_tag` forces this), the weekly
   page server-rendered, JS-off path complete.
5. **Claims** — holds, bookings, the two transactions, drain records,
   the race-loser page. The concurrency tests live here.
6. **Mail** — ICS assembly (unit-tested against fixture calendars,
   verified by hand in Gmail + Outlook before shipping), `mail_job` sweep,
   manage page with all six states.
7. **Home handlers** — event create/delete on drain, the
   booking→event ledger.
8. **`booking.js`** — timezone re-render, split-button confirm, day pager.
   Ship. Point youcanbookme's URL at it.
9. **The poll** — tables, capability URLs, the grid (this is where the
   ARIA/pointer work happens), nudge/close jobs, the ranker, guardrailed
   finalize. Ship the first group poll to a real lab meeting.

Each step is independently landable and testable without the ones after it;
steps 1–3 produce no public surface at all and can run against the live
calendars from day one.

## 8. Open questions

1. **SES + ICS deliverability.** METHOD:REQUEST from SES with
   organizer-on-sending-domain is the documented-correct shape; verify
   against Gmail, Outlook.com, and Exchange tenants before step 6 is
   called done. A test matrix, not an architecture risk.
2. **Sender identity.** `bookings@<gate-domain>` vs a dedicated subdomain
   (`mail.` with its own DKIM). Decide when SES reputation work happens
   (§15's outbound-mail problem, same basket).
3. **Key scope for the slot push.** Publish key (fewer keys) vs a new
   `slots` scope (a stolen publish key today can deface pages; adding slot
   forgery widens it by little, but the scopes exist to be narrow).
   Leaning: new scope, key hygiene is cheap.
4. **Poll deadline defaults** and how hard the nudge cadence caps (one
   nudge feels right; zero-config should do the right thing for a lab
   meeting scheduled four days out).
5. **Does the booking page take a `purpose` select** (Action-Selector
   shape, richer triage) or stay minimal with one free-text topic? The
   incumbents say minimal converts better; the front door says typed
   fields quarantine better. Probably: one select with 3–4 values plus
   optional topic, and measure.
