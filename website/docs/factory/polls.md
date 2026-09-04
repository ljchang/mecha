---
title: Polls
sidebar_position: 7
description: A poll is a list of typed questions — choice, ranking, likert, vas, text, times — answered by ballots the box validates against each question's own vocabulary.
---

# Polls

The scheduling poll asked one question: which of these times can you do? The
general poll is that question generalised — **a poll is a list of typed
questions**, answered by ballots the box validates against each question's own
declared vocabulary, and tallied by pure functions both ends can run.

It lives in the same three crates as everything else on the public surface:
`mecha-manifest` holds the types and the tallies, `factory-publish` creates
polls and reads them back, and `mecha-factory` — the box — serves the pages and
stores the ballots.

## Three rules carry the design

**Only declared vocabulary, only legal shapes.** For every kind but `text`, a
ballot is enum values and small integers the *question itself* declared. That is
what keeps poll pages out of [the front door's
quarantine](/docs/features/frontdoor): there is nothing a respondent can type
into a `likert` answer. `text` is the deliberate exception, and it pays a toll —
see [the prose boundary](#the-prose-boundary) below.

**Ballots, never counters.** Tallies are derived on demand from stored ballots.
Every visibility mode, every edit, the CSV export and any future counting method
fall out of that for free — instant-runoff and Borda are two *reads* of the same
ranking ballots, not two ballot formats.

**Absent is absent.** An unanswered question is no answer — never a default,
never a midpoint. The rule reaches all the way into the widget: an untouched VAS
slider does not submit at all, because a slider parked at 50 that nobody touched
would enter the data as a real opinion. `required` defaults to `false` for the
same reason — forcing a respondent to invent an answer to reach the end is the
midpoint-inventing bug wearing a UI.

## The six question kinds

| `kind` | asks for | the tally leads with |
|---|---|---|
| `choice` | between `min_choices` and `max_choices` of the options | counts per option; 1/1 is single choice, 1/N is approval voting |
| `ranking` | an ordering of the options, partial allowed | **instant-runoff rounds** — who held how many first preferences, who was eliminated, how many ballots exhausted |
| `likert` | one point of a discrete labeled scale | the distribution and the **median**, because a Likert item is ordinal data |
| `vas` | a continuous 0–100 position between two anchors | the distribution over a continuous scale |
| `text` | free prose, `max_length` required | nothing on the projector but a word cloud; the sentences stay on the presenter's screen |
| `times` | the tri-state answer over seeded candidate slots | the ranking, and the auto-book verdict |

Two shapes worth knowing:

- A `likert` question is labeled **either** at every point (`labels`, length
  exactly `points` — a proper Likert item) **or** only at the ends
  (`label_min`/`label_max`), never both.
- A `vas` question's `anchor_min` and `anchor_max` are required fields, because
  an unanchored visual analogue scale measures nothing.
- A `times` question stands alone in its poll. Its candidates arrive from the
  freebusy pipeline and never from the spec — see [the times
  poll](#the-times-poll-is-still-its-own-flow) below.

## Writing a spec

A general poll is a TOML file handed to `polls create --spec`. This is the
mid-semester course survey, which is also the specimen rendered in [the
gallery](/docs/factory/gallery#the-survey):

```toml
title = "Mid-semester feedback"
deadline = "2026-03-06T22:00:00-05:00"

[[questions]]
id = "paper"
prompt = "Which paper should the discussion section take on?"
kind = "choice"

[[questions.options]]
id = "world-models"
label = "World models are enough"
detail = "Chen et al., 2026"
link = "https://example.org/world-models"

[[questions.options]]
id = "affect-probes"
label = "Affective probes in fMRI decoding"

[[questions]]
id = "pace"
prompt = "The pace of the course so far is right."
kind = "likert"
points = 5
labels = ["Strongly disagree", "Disagree", "Neutral", "Agree", "Strongly agree"]

[[questions]]
id = "keep"
prompt = "What is working that we should keep doing?"
kind = "text"
max_length = 300

[results]
show = "after_vote"
identity = "anonymous"

[audience]
kind = "link"
max_ballots = 200
```

`prompt` is optional, because a single-question poll's title already is the
prompt. An option's `link` is **data to show, never a thing to fetch** — it
renders as a link and nothing on either end retrieves it.

:::note[A typo'd key is an error, not a no-op]
`serde` cannot combine `deny_unknown_fields` with the `flatten` that gives us
`kind = "likert"` inline, so the keys each kind accepts are checked by hand
against the raw TOML. A misspelled `anchor_mn` fails at authoring time rather
than silently doing nothing — the same arrangement, for the same reason, as
request-type fields.
:::

## Who may answer

```toml
[audience]
kind = "roster"     # the default
```

A **roster** poll mints one capability URL per participant. You name them at
create time (`--participant "Priya=priya@example.edu"`, repeatable, or
`--roster names.csv` for a class section); the box learns the *names* and never
the addresses. Each person's URL is their identity on the poll, which is what
makes editing an answer, showing "2 of 6 answered", and named results possible
at all.

```toml
[audience]
kind = "link"
max_ballots = 200   # required for a link poll
```

A **link** poll is one shared URL. Dedup is a cookie and an honor system, and
the page says so rather than implying a guarantee it cannot make. `max_ballots`
is required because an open write endpoint has to be priced in advance: a bot
run costs the poll its remaining capacity, never the box its disk.

A link poll takes no `--participant` and no `--roster`; the CLI refuses the
combination rather than quietly ignoring the flags.

## Who sees what

Two enums, both enforced server-side, and both **promises made before the
vote** — so they are fixed at creation. The store refuses edits once a ballot
exists, and there is no setter anywhere.

```toml
[results]
show = "after_vote"     # live | after_vote | after_close | creator
identity = "anonymous"  # named | creator | anonymous
```

`show` decides *when* a voter sees results. `after_vote` is the default by
decision — independent ballots first, the summary as the reward for voting.
`live` is the live-response-product behaviour; `after_close` is the course-eval
behaviour; `creator` sends results to the organizer alone, which is the one that
does presentation choreography (see [Live polls on a
slide](/docs/factory/slides)).

`identity` decides *whose names ride the results*. Left absent it resolves from
the audience — `named` for a roster, `anonymous` for a link — rather than
defaulting blindly. A `link` poll has no names to show, so `anonymous` is both
its default and its only legal value; a spec that says otherwise is refused
rather than silently rewritten, because rewriting a promise is worse than
failing to make it.

:::warning[Anonymous has a floor]
Under **three** respondents an `anonymous` poll withholds the per-option
breakdown and reports only the count. "The one person who strongly disagreed" is
not an aggregate, and a class of five where four have answered is exactly where
anonymity stops being real. Every emitter asks the same question of the same
number.
:::

## The commands

The polls' verbs live in `factory-publish`, the home-side binary that holds the
gate address and the key.

```sh
# A general poll from a spec.
factory-publish polls create seminar feedback-mar --spec survey.toml \
  --roster section-a.csv

# Where it stands. --json is the shape an agent reads.
factory-publish polls status seminar feedback-mar
factory-publish polls status seminar feedback-mar --json

# Freeze the answers, and say what happened.
factory-publish polls close seminar feedback-mar \
  --resolution "Replication it is — projects due the last week of classes."

# One row per ballot, one column per question.
factory-publish polls export seminar feedback-mar --out feedback.csv
```

`create` prints each participant's own URL (or, past twelve people, points at
the CSV), writes a record at `~/.mecha/factory/polls/<poll>.json`, and writes
`<poll>.links.csv` — `name,email,url`, which is what an LMS mail-merge eats.
**Addresses never leave your machine**: the box mints the URLs, home holds the
roster, and mailing the links is your act or your agent's outbox-reviewed one.

`close` takes an optional `--resolution`, rendered at the top of the closed
page. It is Loomio's outcome statement rather than an accountability
requirement, which is why it is optional where [the front door's close
reason](/docs/features/frontdoor) is not: the links people are holding should be
able to answer "so what happened?".

`export` is injection-hardened CSV and is **nameless when the poll is
anonymous** — `anonymous` is a serving policy that reaches the drain and the
export, not just the page.

## Watching one without leaving the session

`mecha tui` has a `/polls` modal, built on the same pattern as `/triggers`,
`/outbox` and `/frontdoor`: every mutation shells out to `factory-publish polls
…`, so there is one implementation per verb and nothing the TUI can do that the
command line cannot.

| key | does |
|---|---|
| `↑` `↓` | move between polls |
| `Enter` | the detail view — the CLI's own output, verbatim |
| `r` | refresh this poll from the gate |
| `c` | close it, with an optional resolution typed inline |
| `e` | export ballots to `~/.mecha/factory/polls/<poll>.csv` |
| `s` | show the projector URL |
| `esc` / `q` | back |

One honest difference from the other modals: **the store of record is on the
gate, not on this machine.** The list is drawn from the local creation records —
who was invited, which the box never learns — and everything live arrives by
driving the CLI. So the modal states its staleness ("as of 14:03:22") and an
unreachable gate is a labelled condition on the row rather than a blank panel.

Text answers surface here on purpose. The presenter's own screen is where prose
belongs; a person reading it in a terminal is the safe context, and nothing
drawn in a modal reaches a model.

## The prose boundary

`text` is the one kind whose answers are prose, and it is treated as prose
everywhere downstream: capped at authoring time by a required `max_length`,
capped again at 10,000 characters whatever the manifest says (an
unauthenticated endpoint plus an uncapped text field is an unbounded write), and
carried as `free_text` — the same class of value [the front
door](/docs/features/frontdoor) hands to an extractor rather than to a
privileged run.

On a projector it does not render as sentences. Anonymous prose on a lecture
screen is an incident with a countdown, so a text question projects as a **word
cloud with a structural guard**:

- words are counted **once per ballot**, so one answer repeating a word fifty
  times scores 1 and nobody can shout their way to 72pt;
- a word reaches the wall only when **two or more different ballots** chose it —
  which keeps a lone troll's word off the screen with no profanity list to
  maintain;
- stopwords and short tokens drop, sizes are five discrete buckets with the
  count in text beside them, and the list is sorted and capped so both ends
  render the same cloud.

The full sentences stay on the presenter's screen and in `status`.

An agent reading a poll through `poll_status` **does** get the text answers, in a
`text_answers` field kept separate from the typed tallies. An earlier version
withheld them and returned counts, on the front door's reasoning; that was wrong
here, because in a poll the prose is the data — "what did people say" is most of
why anyone runs one. What makes returning it safe is the mechanism mecha already
has for other people's words, which is not silence: the tool carries
`openWorldHint`, so the answers arrive marked `untrusted_input` and arm the
interlock exactly as a mail body does. The typed and the written stay in separate
fields, which is what lets an answer summarise the prose without treating any of
it as an instruction.

## Layout, and questions about pictures

Two things are presentation and are deliberately not part of what a question
*means* — a tally must never change because somebody rearranged a page.

**Which way the controls run** is `layout` on the question:

```toml
[[questions]]
id = "format"
layout = "horizontal"     # or "vertical"; omit for each kind's own default
kind = "choice"
```

The default is `auto`, which reproduces what each kind always rendered: a scale
runs across the page, a list of options runs down it. A scale becomes a grid of
equal columns with each label under its control, and collapses to one point per
line on a narrow screen.

**A question can be about a picture**, and so can each option:

```toml
[[questions]]
id = "figure"
prompt = "Which version of Figure 2 should go in the paper?"
media = { src = "/f/fig-all.png", alt = "All three panels side by side" }
kind = "choice"

[[questions.options]]
id = "scatter"
label = "Scatter with a fitted line"
media = { src = "data:image/png;base64,…", alt = "A scatter plot with a fitted line" }
```

Options render as cards you press rather than dots you aim at, with the picture
inside the card and picture options side by side — comparing two figures means
seeing both at once. The radio is still in the markup and in the tab order,
because it is what the form posts, what a screen reader announces, and what works
with the script blocked.

Two rules, both enforced at authoring time:

- **`alt` is required.** A question that asks people to choose between pictures
  is unanswerable without it for anyone using a screen reader, and a poll is a
  thing you send to a group whose eyesight you do not know.
- **`src` is a `data:` URI or a path this origin serves — nothing else.** Every
  page here sends `img-src 'self' data:`, so an image from anywhere else is
  blocked by the browser. That includes your own artifact subdomain, which is a
  different origin. An off-origin `src` is refused when the spec is parsed,
  because the alternative is discovering it from sixty people looking at a page
  with a hole in it that cannot be recalled.

Inline images are capped at 512 KB before base64: a spec travels as one request
body and is stored whole. **There is no upload channel for poll assets yet**, so
today "poll a set of images" means figures small enough to embed. Photographs
from a phone are not, and closing that gap needs an asset endpoint on the box.

## The meeting poll runs itself

`kind = "times"` is the scheduling poll, and it is the one flow here that
does not start from a spec: its candidates are the availability engine's
earliest feasible slots — already minus your real busy time — drawn from the
same slots pipeline that keeps your booking page fresh. From the owner's
chair it is one call, one review, and then nothing until it is booked.

**Ask.** Tell mecha who, how long and what about:

> Find an hour for a lab meeting with Priya and Tal in the next two weeks —
> before the grant deadline, ideally.

The model makes one `poll_meeting_create` call with a title, the participants,
a duration, and optionally a window, a deadline and your sentence. It never
runs a freebusy step or writes a file: the times are drawn at release from
what `slots push` last saw, so a poll staged at five and released the next
morning seeds from the morning's calendar.

**Review.** One outbox card, reviewed as a message: your sentence above the
invitation each person will receive, the recipients, the duration, when
answers close, and the account it sends from. Edit the text if you like;
release it. From the CLI the same thing is

```sh
factory-publish polls create book lab-feb --title "Lab meeting" --duration 60 \
  --participant "Priya=priya@example.edu" --participant "Tal=tal@w.edu" \
  --message "Before the grant deadline, ideally."
```

with no `--policy` and nothing on stdin — the pipeline's cache is the input.

**Wait.** Three deterministic verbs on the timer that already runs the slot
push carry the poll from here, and no model touches any of them:

| verb | does |
|---|---|
| `factory-publish polls sweep` | asks the box for the tally, queues the one nudge, closes the poll on its own terms, decides the verdict |
| `mecha-mail polls` | mails each person their own link from your account, sends the nudge, creates the event for a clean winner — everyone as attendee, re-verified against your live calendar first |
| `mecha polls sweep` | stages the pick card when there is judgment to make, and folds your decision back |

`/polls` in the TUI and `mecha polls list` show one line per poll — *invites
sent*, *needs a pick*, *booked* — and `poll_status` answers "how's
the lab-meeting poll?" with the lifecycle beside the tally.

**Decide.** The policy is *auto with guardrails*, and the numbers are yours
to set in the policy file's `[poll]` table:

```toml
[poll]
auto_book = "unanimous"     # unanimous | feasible | manual
deadline_days = 3           # answers close this many days after the invitations
deadline_hour = 17          # at this hour, in the policy's zone
nudge_hours_before = 24     # one nudge to the silent; 0 disables it
nudge_min_lead_hours = 36   # no nudge when the deadline was closer than this at send
```

The poll closes when everyone has answered or the deadline passes. Under the
default, a time every participant marked plain *yes* — exactly one of them —
is booked by itself: a calendar event from your account with every
participant as attendee, so each receives the provider's native invitation,
and the poll page closes with *"Booked: Tuesday 9 Sept, 2–3pm"* for anyone
who revisits their link. `feasible` books the best-ranked slot even when it
costs someone an if-needed; `manual` never books. Anything the mode does not
book — a tie, a silent participant at the deadline, nothing feasible for
everyone — arrives as **one more outbox card**: a calendar-event draft for the
top-ranked time with the whole ranking and each candidate's reason in its
description. `p` in `/polls` (or `mecha polls pick <poll> <n>`) loads a
different candidate into the same card; release books it; reject closes the
poll as *no time found*. No mode books over someone who never answered.

The candidate list is capped small on purpose: a poll a colleague answers in
ten seconds is the one that gets answered. And while a poll is open its
candidates are holds on your booking page, so a stranger cannot book a slot
your colleagues are still considering.

## Where to go next

- [Live polls on a slide](/docs/factory/slides) — the projector page, and the
  PowerPoint content add-in.
- [Component gallery](/docs/factory/gallery#the-survey) — the survey rendered by
  the code that serves it, open and closed.
- [The front door](/docs/features/frontdoor) — where prose from strangers goes,
  and why a poll's typed answers do not have to.
