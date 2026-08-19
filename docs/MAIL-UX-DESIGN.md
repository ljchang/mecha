# Managing mail from mecha — design

*2026-08-18, revised 2026-08-19. What phases 1–3 settled, and what phases 4–6
will be. `docs/MAIL-UX-RESEARCH.md` is the survey this argues from and
`docs/MAIL-CORPUS-RESEARCH.md` is the measurement that corrected it — **kept out
of the repository** (gitignored, like `OPERATIONS.md`) because its figures are
one person's mailbox rather than a public fact; where any
of the three disagree, the later one wins. Written for review before 4–6 are
built, so anything here is still cheap to change.*

**Revision of 2026-08-19, after a year of real mail was measured.** Two things
changed and both are load-bearing. Front-door routing — the whole of the
original phase 4 — is **dropped**; §1 is now the argument for why, kept rather
than deleted because the reasoning that killed it is worth more than the
proposal was. And the taxonomy was measured instead of guessed, which moved the
largest category into existence and removed one that never arrived.

---

## 0. What is already true

Shipped and running, so these are facts rather than proposals:

| | |
|---|---|
| `mail_triage` | archive / read / unread / spam / trash, closed enum, thread-level, both providers |
| `~/.mecha/mail-triage/` | one typed verdict per thread; ids and envelope metadata, never bodies |
| The classifier | no tools, no history, no system prompt, no shared cache prefix; one isolated call per thread |
| `mecha mail` | `classify` / `list` / `show` / `dismiss` |
| Escalation | snippet first; full body when the verdict is `respond` or names a request kind (~25% of threads) |
| The nightly | `mecha-mail-classify.timer`, 05:30 UTC, Dartmouth only |

Measured on 51 real threads when it shipped: 30 `ignore`, 9 `notify`, 12
`respond`. That sample is superseded by the corpus measurement for anything
about *what arrives*; it remains the only measurement of
what the classifier *decides*, and the 51 stored records predate two taxonomy
changes, so they need a `--force` re-sweep before any number is taken from
them.

### Six decisions that are settled, and should not be re-opened without cause

1. **The privileged run sees the extraction, never the prose.**
   `Record::for_privileged_run` has no argument that returns the subject, the
   sender's display name, the classifier's reasoning, or `one_line`.
2. **Tags are mecha's own**, not Gmail labels or Graph categories. They cost no
   scope and work identically per provider. The price — invisible in every
   other mail client — is accepted.
3. **`mail_triage` is `destructiveHint` alone**: never in `[outbox] tools`,
   never `readOnlyHint`.
4. **Recognising a request kind is not routing it.** `REQUEST_TYPES` is what
   the classifier can name. *(The decoupling holds; what it was preparing
   for — `ROUTABLE_TYPES` and promotion to the front door — was dropped on
   2026-08-19. See §1.)*
5. **The store is an index, not a mailbox copy.**
6. **Google stays in Testing**; CASA revisited after the main features land.

---

## 1. Front-door routing — proposed, then dropped

**This was phase 4 and is now nothing.** The claim was: an email asking for a
letter is a `letter` request that arrived through the wrong door, so promote it
into `~/.mecha/requests/` and let the front door's machinery — quarantined
extractor, `needs-info`, `triage` drafting into the outbox, `reconcile` — carry
it to an answer. The record would be a form request with every field blank.

Five things sank it, and they are recorded because each one is a test worth
applying to the next integration that looks this tidy:

- **The front door's premise does not hold for mail.** Its `[verification]`
  block exists to prove that a stranger controls an email address. An email
  *arrived from* that address. The machinery being imported solves a problem
  mail does not have.
- **Routing removes the thread from the mail queue**, so "did I ever answer that
  student?" needs two places. The queue's entire value is being the one place.
- **It required adding an `origin` field to the front door** — modifying it for
  mail's benefit, which breaks the "the front door has never heard of mail"
  property that was being cited two paragraphs earlier as a virtue.
- **The synthetic record's `values` was empty**, and that was described as "the
  point". It was a degenerate record being rationalised.
- **Manifests live in another repository**, synced by hand. Mail would be
  permanently downstream of a directory it cannot see.

The useful idea underneath survives, and it is much smaller than the machinery
that was attached to it: *some emails are a recognisable kind of request with a
standard set of things you need before you can answer.* That belongs in mail,
natively — mail's own request kinds, mail's own `needs-info` that parks a
thread and names what is missing. No shared type, no cross-repo dependency.

And the relationship to mecha-factory is **substitution, not integration**: if
the form works, the email never arrives. That is the load reduction the whole
exercise is for, and it needs the two systems to cover the same ground from
different sides, not to share a schema.

**Consequences in code:** `ROUTABLE_TYPES`, `is_routable` and
`Proposed::Frontdoor` are deleted. `REQUEST_TYPES` stays, stops mirroring
`mecha-manifest/types/`, and becomes mail's own — which is what §2 measures.

---

## 2. Phase 4′ — the taxonomy, measured

`docs/MAIL-CORPUS-RESEARCH.md` is the full measurement and is gitignored; this
is what it obligates, stated so every decision here stands without it.

A year of mail was fetched raw and deliberately unclassified, because running
it through the current classifier would have projected the eight guessed tags
onto it and confirmed them by construction. Three findings change the design.

### The classifier should not see half of what it sees

About half of all threads — bulk plus a sender-address pattern — are disposable
with no model at all, and across a year the number that were wrongly caught was
a handful. `List-Unsubscribe` alone finds only two thirds of it: it catches
marketing, which is obliged to offer an unsubscribe, and misses every
institutional and transactional sender, which is not. A short regex over sender
and display name closes the gap at a negligible error rate.

**So a deterministic pre-filter runs ahead of the classifier**, and it is the
cheapest change available: half the token cost, and the errors it makes are
measurable against the corpus rather than argued about.

### The vocabulary was wrong in both directions

| Change | Why |
|---|---|
| **add `student-advising`** | The largest single category by a wide margin, and absent from the list. Major plans, prerequisites, petitions, transfer credit, thesis logistics |
| ~~add `finance-admin`~~ | **Rejected on implementation.** The volume is real, but it fails this list's own test: a request kind is one where *a standard set of things must be known before it can be answered*, and nothing has to be gathered before a receipt is forwarded. It is the existing `expense` tag plus `Proposed::Forward` — and the actual gap was that `forward` had no key bound to it, which §4 fixes. Adding a type here would have been the same mistake as `book`, made the same week it was found |
| **remove `book`** | Two threads in ten months, and neither is a request to write a book. A name on this list is a claim that the kind arrives |
| **add `advising` to `TAGS`** | `teaching` did not cover it: a prerequisite question, a major plan and a course petition are advising load, not a class being taught |
| keep the rest | `review`, `letter`, `lab-application`, `speaking`, `meeting`, `data-request`, `grant-support` all appear at real volume |

`student-advising` being invisible to intuition is the finding to remember: it
is the most routine thing that arrives, and routine things do not come to mind
when a person lists what their inbox contains. That is the general argument for
measuring a taxonomy rather than proposing one.

### The metric is reply rate, and the baseline lives outside the repository

The figure is in the gitignored measurement. What matters here is that it is
**low, and measurable without a human grading anything** — which is what makes
it the number every change to this feature is judged against.

**The shape is the finding, not the level.** Most replies that ever happen
happen on the first day, and **a thread still unanswered after a day is
overwhelmingly unlikely ever to be answered**. There is no slow middle to speed
up: mail is handled or it is not, and the decision is made on day one.

That reorders this document. Every other phase here operates on a thread the
user is already looking at — the small fraction that already works — so §4 is
now the highest-leverage item in the plan, and it was an open question until
this was measured.

Peer review is the sharpest case: real volume at the **lowest reply rate of any
category**, against hard deadlines, and the corpus contains the reminder invitations that are what
an unanswered one looks like from the outside.

---

## 3. Phase 4″ — tasks and `needs-info`, native to mail

Promoted from phase 6, because "too many emails and things to do, and I am bad
at keeping track" is the problem statement rather than a refinement of it.

**Built 2026-08-19** as `mecha mail task` and `mecha mail needs-info`; the
modal keys in §5 will drive these rather than reimplement them.

- **`t` carries the deadline.** A thread whose verdict names a due date creates
  the task with that date, rather than a task someone has to re-read the mail
  to schedule. The output says whether the date came from the verdict or from
  the flag, because "it noticed" and "you told it" are different facts about
  how well the classifier is doing.

  Open question 2 is answered by the tool's own schema: `kg_task_create` takes
  `name`, `due`, `context` and `project`, so the thread pointer goes in
  `context` — there is nowhere else. And `project` **must resolve to an
  existing node**, so it is passed through untouched and never invented from a
  subject line: a project conjured out of mail is a board that cannot be
  queried, which is the failure that field exists to prevent.

  The task's `name` defaults to the classifier's `one_line`, which describes
  the mail rather than the action. That is a real mismatch rather than a
  rounding error — hence the override flag, and hence saying so here instead of
  pretending the default is right.
- **`n` parks a thread and names what is missing** — mail's own `needs-info`,
  the surviving half of the front-door idea. The most useful thing mecha can do
  with "can you write me a letter?" is ask the questions that make a good one
  possible.

  **`parked` is a distinct state from `dismissed`, and collapsing them would
  lose the threads most likely to go quiet.** Dismissing says "I am not doing
  this"; parking says "I have asked and cannot proceed yet", and the thread
  stays the user's problem. A request waiting on an answer is exactly the shape
  §2 measures as dying on day one. Parking without naming what is waited for is
  refused, because that is dismissing slowly.
- **Mecha helping complete the task** is the outbox plus an agent run, both of
  which already exist. Nothing new is required for the approval story.

---

## 4. Phase 4‴ — day two

**The highest-leverage thing in this document, and the only item that reaches
the threads that die on day one.** §2 measures the failure as abandonment
rather than mishandling; nothing else here operates after the user has stopped looking at
the queue.

The mechanism needs no new state. The triage store already holds the verdict,
and `mail_search` can say whether an outbound message has gone since. So:

> a thread the classifier called **`respond`**, aged past ~24 hours, with no
> outbound message since — put it back in front of the user, once.

Four things it must get right, three of them from the caveats the measurement
carries:

- **It keys on the `respond` bucket, never on silence.** Silence is not
  evidence of anything on its own.

  **But selectivity is not the goal, and assuming it was is a mistake this
  document made.** The first eval surfaced ~83% of unanswered threads as
  needing action, which was read here as a possible precision problem. Luke's
  correction, 2026-08-19: *"there are many emails that I should have responded
  to — that's why we are trying to build a system to help me with this."* The
  backlog is real, so a system that filtered it down to look manageable would
  be hiding the problem rather than solving it.

  What follows is that day two's job is **not to show less, it is to make
  acting cheaper**. Surfacing seven threads a day with nothing attached is a
  longer to-do list. Surfacing them with a draft ready to approve is the
  feature. That is the outbox and an agent run, both of which already exist,
  and it is what "help me complete tasks and respond with my approval" asked
  for from the first conversation.

  Ordering therefore matters more than filtering: deadline first, then who is
  waiting on an answer, then how long it has waited. A mechanism built on
  "no reply yet" nags about FYIs, and a nudge that fires on everything has
  stopped being a nudge — the same failure as a taint warning that fires on
  every draft.
- **A thread settled elsewhere must be forgivable.** Mail answered in a
  meeting, over Slack, or by somebody else is indistinguishable here from mail
  ignored. So resurfacing is a *question*, not an assertion that the user
  failed, and dismissing one has to be one keystroke and permanent.
- **Once, not repeatedly.** A second reminder for the same thread is how a
  resurfacing surface becomes another queue nobody opens. If a thread is
  dismissed or re-surfaced once and still unanswered, it has been decided.
- **It is a pull surface too, or it changes nothing.** The reason day two fails
  is that mail is only looked at when the user goes looking. So this has to
  arrive somewhere already attended — the morning trigger's briefing, the TUI
  status line, or Slack — rather than being a list that must be remembered.

**The surface is the morning briefing** (decided 2026-08-19). The TUI is
attended and can act, but it only reaches you when it is already open — and on
day two you are by definition not looking at mail, which is the whole failure.
Slack reaches a phone, which is the argument for it and against it: a daily
nudge on a phone is the kind of thing people mute, and a muted nudge is worse
than none. The briefing is already read daily and already runs, so it costs no
new surface and no new habit.

Its limitation is real and shapes the split: **a trigger is unattended and
cannot ask**, so the briefing *lists* and acting happens elsewhere. That makes
the aged set a store-side primitive rather than a feature of the briefing —

> `mecha mail list --aged` — `respond` threads past the age, no outbound
> message since — deterministic, no model, and the briefing is one reader of it.

Which is the same rule the rest of this surface follows: the store and the CLI
are the product, and every front-end is a reader. A day-two list that existed
only inside a trigger's prompt could not be tested, scripted, or read from the
modal that is coming.

**Deliberately not built**: auto-replying, auto-nudging the sender, or any
escalation ladder. This surfaces to the user and stops.

---

## 5. Phase 5 — `/mail`

A sixth modal on the `/outbox` pattern: store read for display, every mutation
a `mecha mail …` child process, slow work spawned detached and watched by
polling the store rather than the child.

```
 ┌ mail ───────────────────────────── 22 need you · 3 drafted · 2 parked ─┐
 │ ● today  dartmouth  #admin       JOCN review — accept or decline       │
 │ ● week   dartmouth  #lab-app     PhD applicant asks about openings     │
 │   week   dartmouth  #rec-letter  Endorsement letter, due Sep 1         │
 │   none   dartmouth  #expense     Amazon receipt, $412                  │
 │ ✎ drafted dartmouth #lab-app     reply staged → /outbox                │
 └ r reply · a archive · s spam · e schedule · t task · f route · ! wrong ┘
```

| key | action | lands as |
|---|---|---|
| `r` | reply | detached agent run → drafts into **`/outbox`** |
| `a` | archive | `mail_triage`, immediate |
| `s` | spam | `mail_triage`, immediate, confirms |
| `e` | schedule | `calendar_create_event`, staged |
| `t` | task | `kg_task_create`, immediate |
| `f` | forward | to a named recipient — staged in **`/outbox`** |
| `g` | tag | edit tags on the record, no model |
| `n` | needs-info | park it |
| `!` | wrong | a correction (phase 6) |
| `enter` | detail | prose, verdict, reasoning |

Three rules carried from `/outbox`:

- **`r`, `e` and `f` are agent runs**, not keystrokes — they build a tool
  surface and can take minutes, so they spawn detached and are watched.
  `a`, `s`, `t`, `g` are single calls and run synchronously.
- **`f` (forward) exists because it was missing.** `Proposed::Forward` was in
  the enum with no key bound to it, so the receipts-to-the-finance-person case —
  one of the five that motivated this whole feature, and real volume by §2's
  measurement — had no way to happen. It took over `f` when routing was
  dropped, which is its natural owner anyway.
- **The result of a reply lands in `/outbox`, not here.** There is exactly one
  approval surface and this is not it. `/mail` decides *whether* something
  needs an answer; `/outbox` decides whether *this* answer goes.
- **`s` confirms.** Spam trains the provider's filter; it is the one triage
  action with an effect outside the mailbox.

---

## 6. Phase 6 — the correction loop

`!` marks a verdict wrong. Three things happen, and keeping them apart is the
design:

1. **The record is corrected in place** — a typed before/after pair on the
   store. Free, deterministic, and immediately useful to the list.
2. **The pair joins a few-shot pool** the classifier's prompt draws from.
   Cheap, fast-acting, small blast radius: it steers a tool-less pass that
   emits a fixed schema. **Built 2026-08-19.** Bounded at `FEW_SHOT_MAX`
   examples, chosen by recency rather than relevance — picking the most
   *similar* examples would need a similarity measure over mail the classifier
   has not read yet, which is expensive and is a second place for a scoring
   function to be quietly wrong.

   It is also **the one place a thread influences another thread's verdict**,
   and that is worth stating because the rest of the classifier maintains the
   opposite: one call, no history, nothing an email says can reach the verdict
   on a different email. Three things keep the exception narrow — a human must
   have corrected the thread for it to appear at all, the examples are fenced
   as data with the warning *before* them, and the typed change is the payload
   while the prose is only enough to say what kind of mail it was.
3. **A `triage`-domain reflection** goes to the learning store. **Built
   2026-08-19**, and two things about the path differ from the ordinary one.

   Provenance: the lesson is argued from mail, so it is recorded
   `Origin::Untrusted` honestly rather than laundered to `Clean`.
   `Reflexion::learnable` admits it because triage rules reach only the
   classifier — see `LEARNING-AUTONOMY-DESIGN.md` §4 — not because the origin
   was fudged.

   And there is no proposal gate any more; the ledger is `mecha mail score`.
   A rule that starts burying answered mail regresses a measured number and is
   retired, which is what makes accepting it without a human safe.

   **Declining is the common case and the frame says so.** Most corrections are
   judgements about one moment rather than a pattern, and a rule rides in every
   future classification while a missing rule costs one verdict. A reflector
   that answers unparseably is a *failure*, not a decline: the correction stays
   unmined so a later pass retries it, on the same reasoning that stopped a
   failed classification burying a thread forever.

### Why the few-shot pool is not a learned rule

A learned rule rides in every future run's cached prefix, which is why
`learning.rs` gates provenance so hard. A few-shot example injected into a
tool-less classifier that returns a fixed schema is a far smaller thing. Fusing
them would mean either over-gating the cheap mechanism into uselessness or
under-gating the expensive one, and the second is how third-party text reaches
every future prompt.

`triage` is therefore **not** in `RUN_DOMAINS` — its rules ride in the
classifier's own frame and nowhere else. That separation is why domain
selection was built before this phase existed.

### What a correction records

`{ thread_id, account, field, was, now, at }` — field-level, because "wrong" is
not one thing. A misread bucket, a missed deadline and a wrong `request_type`
are different errors with different fixes, and a correction store that flattens
them teaches the classifier noise.

---

## 7. Open questions, for review

1. **Should `r` (reply) hand the drafting run the thread, or the verdict?**
   The thread is what an answer is written from, and taking the taint honestly
   is the design elsewhere. But it means every reply run is trifecta-armed and
   its draft comes out red in `/outbox`. Leaning: hand it the thread, accept
   the red, because the alternative is drafting a reply from a summary.
2. **Does `t` (task) attach the thread id to the task?** pkg's
   `kg_task_create` takes `name`, `due`, `context`, `project`. A pointer back
   to the mail would make "why is this on my board" answerable, but there is no
   field for it — it would have to live in the name or need a pkg change.
3. ~~**Where does day two surface?**~~ **Decided 2026-08-19: the morning
   briefing**, with the aged set as a store-side primitive so the briefing is
   one reader rather than the owner. See §4. What is still open underneath it
   is the *age* — 24 hours is where the cliff falls, but a briefing that fires
   the morning after an evening email would be nagging about something a
   working day old. Likely one working day rather than 24 hours, and that is
   worth measuring against the corpus rather than picking.
4. **`meeting` as a request kind** — still unresolved, and now with a number
   on it: real volume, and the *highest* reply rate of any category, which cuts
   both ways — it arrives often, and it needs help least. It remains structurally the greediest label — almost any request can
   be discussed in a meeting — and the booking flow may already cover it.
5. **How is `student-advising` answered?** It is the largest category by a wide
   margin and the design had said nothing about it, because it did not know it
   existed. Much of it is the same handful of
   questions — prerequisites, petitions, transfer credit — which is the profile
   of something a form or a published answer removes rather than something
   mecha should answer one at a time. That is the `mecha-factory` substitution
   argument from §1 landing on the biggest single piece of the load, and it
   deserves its own pass before any of it is automated.
6. **Retention.** Nothing prunes `~/.mecha/mail-triage/`. A year of nightlies
   is perhaps 15k small files. `mecha work clean` has a policy shape worth
   copying, but an archived verdict is also the eval fixture and the few-shot
   pool, so deleting has a cost the work directory does not have.

---

## 8. What is deliberately not being built

- **Auto-send of anything**, at any confidence, including the finance forward.
- **An autonomy tier.** Inbox Zero's per-rule graduation is the good version
  and it is v2 at the earliest; it should follow `/review now|later|auto`'s
  rule — set by explicit command, never inferred.
- **A second approval surface.** The outbox is it.
- **`mail_snooze`.** Neither provider has one; it would be a label plus a
  trigger, and a snooze that silently means "labelled and forgotten" is the
  silently-degrading-sandbox shape.
- **A mail cache or local index.**
- **Manifests invented from email traffic** without a human writing them.
