# Polls — generalising the group poll past scheduling

*2026-08-08/09. Grows §5 of SCHEDULING-DESIGN.md into a general-purpose
poll: opinions on anything, not only meeting times. Grounded in
POLL-RESEARCH.md — the survey of what StrawPoll, Doodle, Rallly, OpaVote,
RankedVote, Discord, Loomio, Mentimeter and Framadate actually ship. The
requirements are the user's: question kinds multiple-choice, Likert, VAS,
ranking, and capped free text (the middle two from the lab's instrument
tradition rather than consumer polling, which stops at generic rating
scales); large anonymous surveys for taught courses, which is what makes
a poll a* list *of questions and adds the open-link audience; and
PollEv-style live lecture polling — the projected screen and the
presenter-controlled reveal, one URL per question by choice (§3.1).*

The scheduling poll shipped with the right bones and the wrong nouns. It
already stores **ballots, not counters** — a per-participant answer map,
which the research names as the one structural bet that separates a poll
component from a counter widget, because every tally method, edit,
visibility mode and export derives from ballots. It already has the
strongest practical integrity rung (per-participant capability URLs — the
single-use-code model OpaVote uses for real elections, without an account
system). What is scheduling-specific is only the *vocabulary*: candidates
are time slots, answers are the tri-state, and the tally is `rank_poll`.

The one-sentence shape, and everything below serves it:

> **A poll is typed questions out, typed ballots back, and a visibility
> policy the box enforces.** Home decides the questions and reads the
> verdict; the box collects ballots it validated against each question's
> own schema and serves each viewer exactly what the policy allows them —
> never more, never client-side.

One kind breaks the "typed" half on purpose. Five of the six answer
vocabularies are enums and small integers the origin validated, so those
answers never feed the quarantine at all. The `text` kind is the
exception, and it is handled as an exception: its answers are stranger
prose, capped at the schema, escaped at render, and drained as
`free_text` through the same extraction pass a booking topic already
rides. The front-door sentence — *the privileged run sees the extraction,
never the prose* — holds unchanged.

---

## 1. The schema — a poll is a list of questions

A poll carries `questions`, plural, **from day one**. Most polls have
one; a course survey has eight; the ballot is a map of question id →
answer either way, so the single-question poll is a list of length one
rather than a special case that a later migration has to unwind. What
stays refused is *branching* — every participant sees every question in
the same order (§9). `polls.candidates` becomes this typed structure,
defaulted on load so every existing row is what it always was (one
`times` question).

```toml
# The general shape, as `polls create --spec` reads it.
title = "Which paper should lab meeting discuss?"
deadline = "2026-08-13T17:00:00-04:00"

[[questions]]
id = "paper"             # slug rules as poll ids: [a-z0-9-_]
prompt = "Pick one."
kind = "choice"          # choice | ranking | likert | vas | text | times
min_choices = 1          # 1/1 = pick one; 1/N = approval;
max_choices = 1          #   the cap is StrawPoll's "choose up to N"

[[questions.options]]
id = "world-models"
label = "World models are enough"
detail = "Chen et al., 2026"    # optional, one line
link = "https://arxiv.org/abs/2606.01234"   # optional, shown as a link —
                         # data to show, never a thing to fetch,
                         # the Url field's rule verbatim

[results]
show = "after_vote"      # live | after_vote | after_close | creator
identity = "named"       # named | creator | anonymous
                         # defaults: after_vote + named (§5)

[audience]
kind = "roster"          # roster | link   (§3)
```

The six question kinds:

- **`choice`** — options with `min_choices..=max_choices` selections.
  Answer: a set of declared option ids.
- **`ranking`** — options ranked. Answer: an ordered list of distinct
  declared ids; partial rankings legal (rank your top three of six).
  Tally: **IRV**, with Borda scores as a second read of the same ballots —
  consumer products converge on IRV almost universally, and separable
  ballot/tally means shipping both reads costs one pure function each.
- **`likert`** — a discrete labeled scale applied to the prompt's
  statement: `points` (2–11), and either a full `labels` array (length
  must equal `points` — a proper Likert item labels every point) or
  `label_min`/`label_max` endpoints with numbered interior. Answer: one
  integer in `1..=points`. Likert data is **ordinal** — the honest
  summary is the distribution and the median, and that is what the tally
  leads with (the mean is emitted too, labelled as the courtesy it is).
- **`vas`** — a visual analogue scale: a continuous 0–100 line whose
  `anchor_min`/`anchor_max` are **required**, because an unanchored VAS
  measures nothing. Answer: one integer in `0..=100`. Treated as
  interval data: means are defensible, and the tally emits mean, median
  and a decile histogram.
- **`text`** — a free response to the prompt. `max_length` is
  **required**, the `FieldKind::Text` rule for the same reason: this is
  an unauthenticated endpoint and an uncapped field is an unbounded
  write. Answer: one string. There is no tally — results are the
  responses themselves, listed.
- **`times`** — exactly today's poll: `timezone`, `duration_minutes`,
  slot candidates, tri-state answer map. The week grid, the paint
  interaction, `rank_poll`/`clean_winner`, the hold coupling — all
  unchanged, now one kind among six instead of the whole feature. A
  `times` question does not share a poll with other kinds (nothing
  forbids it in principle; nothing asks for it, and the page layout
  assumes it stands alone).

Per-question `required` defaults to false: a survey answer skipped is an
answer withheld, and forcing a student to invent a rating to reach the
submit button is the midpoint-inventing bug (§2) wearing a UI. The ballot
stores what was answered; the tally counts per-question n.

Validation is the "only the three words" rule generalised: the POST
accepts **only declared vocabulary, only legal shapes** — a question or
option id never declared, a selection count outside the cap, a rank list
with a repeat, an integer off the scale: ignored or refused, never
stored. For every kind but `text`, the questions define the ballot's
entire vocabulary, so nothing a voter sends can be anything but enums and
small integers. A `text` answer is validated for the only things a
schema can say about prose — its length, its being valid UTF-8 — and is
*treated as prose* everywhere downstream (§4).

Prompts and options are the organizer's words, pushed from home — but
they are still rendered into HTML the box serves, so they are escaped
like every other manifest string, and `link` is constrained to http(s) at
validation like the form renderer's URL handling.

**Polls become first-class, and the instrument becomes an argument.**
Today `polls create` takes an instrument because a times poll's candidates
must hold slots on the booking page. That coupling is real but belongs to
the `times` kind alone: a general poll names no instrument, and a times
poll says `--holds book` explicitly. The URL space (`/p/<handle>/<id>/…`)
never mentioned the instrument anyway.

**Where the form/poll boundary now sits.** The request form remains the
intake product: verified identity, individual submissions triaged at the
front door, one direction. A poll is the feedback product: a defined
audience, aggregate results, anonymity modes, results optionally served
back. Multi-question no longer divides them — *branching, verification,
and triage* do.

## 2. Ballots — storage, editing, integrity

Unchanged where it was right:

- **The capability is the identity.** Per-participant URLs, hashed at
  rest, no name typing (when2meet's typo'd-duplicate failure stays dead).
  The box knows names, never emails; home's local record keeps the join.
- **The newest submission replaces the last** while the poll is open —
  every scheduling tool allows editing and the deadline enforces the end.
  X and Reddit's no-edits model buys nothing here. Autosave is
  per-question, so a half-finished survey is saved as far as it got.
- **Absent is absent.** For `times`, silence stays "no" (when2meet's
  rule). For every other kind an unanswered question is *no answer* — a
  Likert item where silence counted as the midpoint would be inventing
  data. The same rule reaches inside the `vas` widget: an untouched
  slider is not a 50 (§6).

## 3. Audiences — the roster, the roster at scale, and the open link

**`roster`** (the default) is today's model: named participants, one
capability URL each. For a lab it is mailed by the agent through the
outbox, unchanged.

**The roster scales to a course section** before any new mechanism is
needed: `polls create --spec survey.toml --roster students.csv`
(name,email — or id,email; the box never sees either email) mints one
token per row and writes `links.csv` beside the local record. What
changes at 150 participants is *distribution*, not machinery — mailing
150 links through the user's own account is a deliverability and budget
problem, so the CSV is the product: it uploads into an LMS mail-merge,
which is the tool that already owns messaging a class. Per-student links
are how real course evaluations work, and for the same reason: completion
is trackable (and nudgeable) while content stays anonymous under the
identity policy — "we can see that you responded, never what you said,"
stated on the page in exactly those words.

**`link`** is the open door the course case actually asks for: one
shared URL posted to the LMS, no roster.

```toml
[audience]
kind = "link"
max_ballots = 400        # required: the hard cap is the abuse story
```

The honest design, since every integrity rung below a personal token is
weak (POLL-RESEARCH.md §4 — the StrawPoll proxy-bot economy is the
measured cost):

- **First visit mints an anonymous ballot capability** into a cookie —
  so edits and autosave work within that browser — and the shared URL
  itself never accepts a ballot directly. Dedup is therefore
  per-browser: honor-system, evadable in incognito, and the page says
  "one response per person, on your honor" rather than pretending.
- **`identity = "anonymous"` is forced.** There is no roster, so there
  are no names worth showing, and asking visitors to type one is
  when2meet's typo'd-duplicate bug returning as a feature.
- **`max_ballots` is required, not defaulted.** An open write endpoint
  on the gate is priced in advance: a bot run costs the poll its
  remaining capacity, never the box its disk. The gate's existing rate
  limiting covers the request path itself.
- **A `link` poll still belongs to a handle** and rides the same
  lifecycle, drain, and visibility machinery — it is an audience mode,
  not a second poll system.

When integrity matters more than distribution convenience (a graded
anything, a contested vote), the roster CSV is the answer; the page
footer of a `link` poll does not claim otherwise.

### 3.1 The classroom screen — live polls in lecture

The PollEv/Mentimeter use: students join at one URL, the instructor
activates a question, a projected chart moves as votes land, and results
appear to the room when the instructor decides. This is presentation
choreography over the machinery above, and it decomposes into three
pieces, only one of which is new:

**No stable class channel — each poll is its own URL, by the user's
call.** A lecture's questions are created as a small deck before class,
each a `link` poll with its own address; there is no series object, no
activation pointer, no machinery for phones to follow. Putting a
question in front of the room means putting its join URL on the slide
(short handle and poll ids earn their keep here) — the PollEv stable
channel is a convenience this deliberately does not buy, at the price of
one more noun and a second pointer store it does not have to maintain.

**The screen view is the reveal.** `/p/<handle>/<poll>/screen/<token>`
— a creator capability minted at create — renders results only,
big-type, no form, the join URL printed large across the top,
refreshing at 2s (one projector is one client; the interval can afford
lecture-speed). Project it beside the slides, or iframe it from web
slides (reveal.js/Quarto — it is just a themed page); a browser window
on the second display is the first-lecture story and the permanent
fallback. Native PowerPoint embedding is no longer out of scope: a
**content add-in** — a static sideloaded manifest plus one wrapper page
the box serves, a third consumer of this same screen URL — is step 10
of the build order, its shape recorded in SLIDES-RESEARCH.md §3
(decided 2026-08-09, after the survey of how Mentimeter/PollEv/Slido
actually ship). Keynote gets nothing to build, by evidence rather than
neglect: it has no add-in model, and the browser window (or the OBS
Live Video pipe documented in that survey) is the story there.
Advancing to the next question is switching to
the next poll's screen — browser tabs lined up before class. Set the
polls `show = "creator"` and the choreography completes itself: student
phones show the ballot and never the results, the room sees results
exactly when the screen is on the projector — **presenter-controlled
reveal with no new enum**, the existing policy doing the work.

**A QR generator is a future day's presenter-side tool.** A hall joins
faster by camera than by typing, but the QR is an artifact that goes
*on the slide* — a `polls qr` CLI verb emitting an SVG, nothing the box
or the pages need to know about. Noted so it is not forgotten; designed
when wanted.

**The presenter's controls are the TUI monitor** (§7): watch the
response count climb, close the poll — every action a
`factory-publish …` child process, the `/triggers` pattern.

Two boundaries stated up front:

- **Text sentences do not auto-project; recurring words do.** Anonymous
  prose on a lecture screen is an incident with a countdown; every
  live-response product ships moderation for exactly this. The screen
  renders typed tallies freely, and text questions project as a **word
  cloud with a structural guard** (the user's call, reversing the
  original word-cloud exclusion): words are counted once per ballot and
  reach the wall only when **two or more ballots** chose them — a lone
  troll's word never renders, with no profanity list to maintain.
  Stopwords drop, sizes are five discrete buckets with the count in
  text (the heat-cell rule), and the full sentences stay on the
  presenter's own screen and in `status`. A projected-sentence mode
  still waits for a moderation queue worth building.
- **Participation credit needs no new machinery.** If responding counts
  toward a grade, that is the roster CSV (§3): completion visible,
  answers still `anonymous` — "we can see that you responded, never
  what you said" is precisely the course-eval contract. A `link` series
  cannot grade participation and the docs say so rather than hedging.

## 4. The `text` kind crosses the prose boundary, and pays the toll

The other five kinds stay outside the quarantine by construction. `text`
cannot, so it follows the booking topic's path exactly rather than
inventing a second rule:

- **On the box**: capped at `max_length` before storage, escaped at every
  render, newlines kept but runs bounded — stranger prose on a page we
  serve, handled the way `Confirmation::render` already handles the
  submitter's own words.
- **Between participants**: showing responses to other *people* under the
  `results.show` policy is the safe context — the front door's `show`
  verb already prints a stranger's prose for a human reader, because a
  person in a terminal (or a browser) cannot be prompt-injected into
  exfiltrating a calendar. Escaping is the whole defence humans need.
- **Toward the agent**: the drain marks every `text` answer `free_text`,
  and a privileged run sees only the extraction — never the prose, never
  a paraphrase of it. `status --json` separates the two: typed tallies
  ride plainly; text answers ride in a clearly-marked prose section the
  agent-side tooling routes through the extractor before any run with
  tools reads it. A course survey's 150 free-text comments are exactly
  the volume this pass exists for.
- **Anonymity is weaker here, and the page says so.** Stripping the name
  off a paragraph does not anonymise the paragraph — prose identifies
  its author by style and content. An `anonymous` text question states
  "your name is not recorded with your answer" and does not pretend
  more than that.

What `text` is **not** is write-in options. A text answer is prose shown
as prose; a write-in is a stranger's words becoming an *option other
people vote on* — structure, not content. Write-ins stay excluded (§9).

## 5. Visibility and identity — two enums, enforced server-side

The research's clearest finding: these are two independent axes that
products conflate, and both are promises made to a voter *before they
vote*, so they are per-poll settings fixed at creation — never editable
after the first ballot exists, because a promise you can revoke
retroactively is not a promise.

**`results.show`** — when a *voter* sees results:

| value | meaning | genre norm |
|---|---|---|
| `live` | results visible from the first page load | Doodle, Rallly, Discord — and today's heat counts |
| `after_vote` | results appear once your ballot is in | X, Reddit — kills the bandwagon/anchoring effect |
| `after_close` | results appear when the poll closes | StrawPoll option, Mentimeter's reveal |
| `creator` | voters never see results; home does | StrawPoll "keep hidden" — and the course-survey norm |

**`results.identity`** — whose names ride the results:

| value | meaning |
|---|---|
| `named` | the Doodle grid: everyone sees who answered what |
| `creator` | Doodle's hidden poll: voters see aggregates only; `status` shows home the names |
| `anonymous` | aggregates only, *home included*: `status` and the drain carry ballots stripped of names |

Defaults, decided: **`after_vote` + `named`** — cast your ballot, see
the summary. The user's call, and the research backs it: independent
ballots first (no anchoring on a live count of two), then the summary as
the immediate reward for voting, which is also the reveal moment the
reactive layer renders best (§6). The exception is `times`, which
defaults `live`: a scheduling grid is coordination, not opinion — the
heatmap *is* the point and watching it converge is the feature. A `link`
audience forces `anonymous` (§3).

Three rules that are each a bug if undone:

- **The policy is enforced where the bytes are emitted, never in the
  client.** The rendered page and the results endpoint both consult
  (policy, viewer, poll state) and omit what the viewer may not see.
  Nothing hidden ever reaches the browser to be un-hidden — "the server
  can only return what the policy allows" is the same direction of trust
  as the box subtracting slots.
- **`anonymous` is a serving policy, and the page says exactly that.**
  Editing your own ballot and one-ballot-per-person both require the box
  to key ballots by participant — every real product works this way
  (Mentimeter, Simple Poll; the research is explicit that edit-plus-
  anonymous forces it). So the promise is: identities are never
  rendered, never served, never drained, never exported — not that the
  database cannot join them. The voter-facing line under the form states
  the mode in plain words, because the trust asymmetry between `creator`
  and `anonymous` is real and a page that leaves it ambiguous is lying
  to somebody. The caveat scales *down* with class size: at n = 150 an
  aggregate distribution genuinely hides an individual in a way a
  twelve-person lab poll cannot, and small-n cells are the reason
  `status` suppresses per-option breakdowns below a floor (default
  n < 3) on anonymous polls rather than printing "the one person who
  strongly disagreed."
- **What `status --json` emits obeys the same policy.** An `anonymous`
  poll drains ballots without participant names — the agent at home can
  tally, and cannot un-anonymise. Response *rate* stays visible in every
  mode (who has answered is lifecycle, not ballot content; Doodle's
  hidden polls and every course-eval system do the same).

## 6. The page — schema components, reactive on top

One page for the whole poll, questions in declared order, in the booking
page's tradition: server-rendered from the schema, theme-tokened,
**JS-off path complete first**, `poll.js` an enhancement layer. A course
survey of eight questions is still one page — steps and progress bars
belong to the form product; a poll page you can see the whole of is a
poll page you finish. Per kind:

- **`choice`** — an option list; radios (`max_choices = 1`) or checkboxes
  with a live "n of m" counter. Results, when the policy shows them:
  horizontal bars per option with counts, rendered server-side.
- **`ranking`** — JS off: a rank `<select>` beside each option
  (Framadate's degraded shape); server enforces distinctness. JS on:
  drag-to-reorder with keyboard move-up/move-down buttons and a polite
  live region — the poll grid's ARIA discipline applied to a list.
- **`likert`** — a labelled radio row, which is already its best form:
  every point a real `<input type="radio">` with its label, JS-off
  perfect, screen-reader native. `poll.js` adds nothing but the
  autosave. Results: a distribution bar per point, the median marked.
- **`vas`** — the one widget where the measurement instrument constrains
  the implementation. JS on: an anchored track whose thumb **does not
  exist until first touch** — a slider with a default position anchors
  responses to it, and an untouched control submitting 50 would be the
  midpoint-inventing bug §2 forbids. Only a touched slider arms the
  autosave. JS off: a plain `<input type="number" min="0" max="100">`
  beside the printed anchors — a native range input cannot express
  "untouched", so the degraded path swaps the widget rather than the
  meaning. Results: mean, median, and a decile strip.
- **`text`** — a `<textarea>` with `maxlength`, a live character counter
  under it (the limit is the organizer's, so the page shows it rather
  than surprising at submit), server-side cap enforced regardless.
  Results: the responses as an escaped list, newest last, each row
  carrying its name only under `identity = "named"`.
- **`times`** — the existing grid and paint layer, untouched.

**Reactive means the results move while you watch.** The mechanism is the
one the booking page already proved with `slots.json` — poll a small JSON
truth, reconcile the DOM, no framework, no websockets:

- `GET /p/<handle>/<poll>/<token>/results.json` — token-scoped, because
  visibility is per-viewer (`after_vote` depends on *your* ballot being
  in; a `link` poll's cookie capability serves the same role). Returns
  exactly what the policy allows this viewer now: nothing, or
  aggregates, or the named grid. `Cache-Control: no-store`.
- `poll.js` refreshes on an interval (10s), on `visibilitychange`, and
  **immediately after its own autosave 204** — the research's one line
  about feel: a voter who just voted sees their bar tick up now, not at
  the next interval. Bar widths transition in CSS; counts change in
  text. The autosave machinery (debounce, in-flight collapse, fall back
  to the button) is already written and carries over whole.
- `after_vote` reveals are a page-state change, not a client decision:
  the 204 means the ballot landed, the next `results.json` fetch is the
  first one the server answers with data, and the results section slides
  in. JS off: the post-redirect-get page shows results, same policy,
  same server.

Live results per kind, honestly scoped: `choice` bars and `likert`/`vas`
distributions update live and mean what they show. `ranking` shows live
*first-preference* counts only, because IRV rounds on a partial
electorate imply a winner that one more ballot can flip — the full
round-by-round elimination table (RankedVote's differentiating display)
renders once, on the closed poll, where it is true. `text` responses
under `live` append as they arrive — the one place the reactive layer is
inserting stranger prose into the DOM, so `poll.js` inserts it as text
nodes, never markup, the same discipline as the escape at render.

## 7. Close, resolution, and the verdict at home

States stay `open → closed`, deadline or `polls close`, close terminal.
Four generalisations:

- **A closed poll carries a resolution.** Loomio's outcome statement,
  adopted: `polls close --resolution "We're reading the world-models
  paper Thursday."` renders at the top of the closed page, so the link
  people hold answers "so what happened?" instead of dead-ending at a
  frozen tally. For `times` polls the booked slot is the resolution,
  which the finalize path already knows how to say.
- **Tallies are pure functions in mecha-manifest**, beside `rank_poll`:
  `tally_choice`, `tally_ranking` (IRV rounds + Borda from the same
  ballots), `tally_likert` (distribution, median, the labelled mean),
  `tally_vas` (mean, median, deciles) — computed per question. `text`
  has no tally, which is itself the honest answer. The box uses them to
  render, `status --json` at home emits their output beside the
  policy-filtered ballots (per-question n included, since `required`
  is false), and the guardrail stays testable the way `clean_winner`
  is. The agent still owns judgment: a clean verdict can auto-resolve
  where the design already auto-books, anything ambiguous stages through
  the outbox with reasons attached.
- **`polls export --csv`** — one row per ballot, one column per
  question (ranking as `first>second>third`, `times` as one column per
  slot), for the instructor-in-a-spreadsheet case. It obeys the
  identity policy like every other emitter: an `anonymous` poll exports
  no name column. Text answers are included (a human in a spreadsheet
  is the safe context, the front door's `show` rule) — but
  **CSV-injection hardened**: any cell starting with `=`, `+`, `-`,
  `@` or a control character is prefixed with `'`, because a student
  answer of `=HYPERLINK(...)` executing in Excel is stranger prose
  reaching a code path, the exact shape every other boundary here
  exists to stop.
- **The TUI grows a `/polls` monitor**, on the `/outbox`/`/frontdoor`
  pattern: the list view shows every open poll with response count
  against roster size (or ballot count against `max_ballots`) and
  deadline distance; the detail view shows per-question tallies
  rendered from the same `status` output the CLI prints — never a
  second tally implementation. Every mutation — close with resolution,
  export — is a `factory-publish polls …` child process, so the TUI can
  do nothing the command line cannot, and it doubles as the lecture
  controller (§3.1). Reading the gate over the network is the one
  novelty against `/triggers` (whose store is local), so the monitor
  states staleness honestly — "as of 12s ago" — and an unreachable gate
  is a labelled condition, not a blank panel.

Creation follows the times poll's path minus the pipeline: `polls create
--spec poll.toml` validates against the manifest types and pushes; the
freebusy pipeline remains the way a `times` spec gets its candidates.
Inviting a roster is the model acting on third parties, so it stages
through the outbox as one reviewable item — unchanged, and unchanged on
purpose: nothing about a general poll touches the send path. A
roster-CSV or `link` poll needs no send at all, which for a course is
the point.

## 8. Worked examples

Eight specs — one per kind, plus the course survey that exercises the
list and the in-class check that exercises the series:

```toml
# 1. The lab-meeting paper vote — the everyday case.
title = "Which paper should lab meeting discuss?"
deadline = "2026-08-13T17:00:00-04:00"
[[questions]]
id = "paper"
kind = "choice"
min_choices = 1
max_choices = 1
[[questions.options]]
id = "world-models"
label = "World models are enough"
link = "https://arxiv.org/abs/2606.01234"
[[questions.options]]
id = "affect-probes"
label = "Affective probes in fMRI decoding"
link = "https://arxiv.org/abs/2607.05678"
# [results] omitted: the defaults — vote, then see the summary, names on
```

```toml
# 2. The meeting-time poll — today's feature, now a spec like any other.
title = "Lab meeting, week of Feb 9"
[[questions]]
id = "when"
kind = "times"
timezone = "America/New_York"
duration_minutes = 60
# candidates arrive from the freebusy pipeline, exactly as today
[results]
show = "live"
identity = "named"      # the only honest setting for scheduling
```

```toml
# 3. The anonymous pulse check — a proper Likert item.
title = "Lab workload check"
deadline = "2026-08-15T12:00:00-04:00"
[[questions]]
id = "workload"
prompt = "My current workload is sustainable."
kind = "likert"
points = 5
labels = ["Strongly disagree", "Disagree", "Neutral", "Agree", "Strongly agree"]
[results]
show = "after_close"    # nobody anchors on a live median of two answers
identity = "anonymous"  # stated on the page in plain words
```

```toml
# 4. The affect probe — continuous, anchored.
title = "Grant-deadline stress"
[[questions]]
id = "stress"
prompt = "How stressed are you about the grant deadline right now?"
kind = "vas"
anchor_min = "Not at all"
anchor_max = "Extremely"
[results]
show = "after_close"
identity = "anonymous"
```

```toml
# 5. The ranked pick — several options, preferences matter.
title = "Name the new cluster"
[[questions]]
id = "name"
kind = "ranking"
[[questions.options]]
id = "hopper"
label = "hopper"
[[questions.options]]
id = "lovelace"
label = "lovelace"
[[questions.options]]
id = "mecha-prime"
label = "mecha-prime"
[results]
show = "after_close"    # a live IRV winner flips; show it once it's true
identity = "creator"
```

```toml
# 6. The capped free response — prose, and priced accordingly.
title = "One question you'd want us to ask the visiting speaker."
deadline = "2026-08-20T09:00:00-04:00"
[[questions]]
id = "ask"
kind = "text"
max_length = 280
[results]
show = "live"           # the shared list is the point
identity = "anonymous"  # names off the list; the page notes prose still
                        # reads like its author
```

```toml
# 7. The mid-semester course survey — the list, at class scale.
title = "PSYC 60 — mid-semester feedback"
deadline = "2026-10-16T23:59:00-04:00"

[[questions]]
id = "pace"
prompt = "The pace of the course so far is:"
kind = "likert"
points = 5
labels = ["Much too slow", "Too slow", "About right", "Too fast", "Much too fast"]

[[questions]]
id = "labs-useful"
prompt = "The lab sessions help me understand the lectures."
kind = "likert"
points = 5
labels = ["Strongly disagree", "Disagree", "Neutral", "Agree", "Strongly agree"]

[[questions]]
id = "confidence"
prompt = "How confident do you feel about the material right now?"
kind = "vas"
anchor_min = "Not at all confident"
anchor_max = "Completely confident"

[[questions]]
id = "keep"
prompt = "What is working that we should keep doing?"
kind = "text"
max_length = 500

[[questions]]
id = "change"
prompt = "What one thing would you change?"
kind = "text"
max_length = 500

[results]
show = "creator"        # the course-eval norm: students answer, the
identity = "anonymous"  #   instructor reads aggregates and quarantined prose

[audience]
kind = "link"           # one URL on Canvas
max_ballots = 400
```

```toml
# 8. The in-class concept check — one of a lecture's deck, on the series.
title = "Week 3: which brain region?"
[[questions]]
id = "region"
prompt = "Fear conditioning most depends on which structure?"
kind = "choice"
min_choices = 1
max_choices = 1
[[questions.options]]
id = "amygdala"
label = "Amygdala"
[[questions.options]]
id = "hippocampus"
label = "Hippocampus"
[[questions.options]]
id = "vmpfc"
label = "vmPFC"
[[questions.options]]
id = "insula"
label = "Insula"
[results]
show = "creator"        # phones never show the answer; the projector
                        #   is the reveal (§3.1)
[audience]
kind = "link"           # this question's URL goes on the slide
max_ballots = 400
```

## 9. Left out on purpose

Each of these is a refusal with a reason, not an omission:

- **Branching and skip logic.** Multi-question is in (§1); *conditional*
  question flow is where a poll becomes a survey engine, and the request
  form already owns conditions (`show_when`) for the intake shape. Every
  participant in a poll sees the same questions — which is also what
  keeps the results comparable, the tally per-question, and the page one
  page.
- **Write-in options** (StrawPoll's "voters add answers"). Distinct from
  the `text` kind (§4): a write-in is a stranger's words becoming an
  *option others vote on* — structure, not content — and it would put
  stranger prose inside the question schema itself. The shape that would
  preserve the invariant, if ever wanted: a proposal drains as
  `free_text` through the extraction pass, and *home* re-pushes the poll
  with the option added — the organizer's push stays the only way words
  become options. Deferred, not designed.
- **Comments.** The `text` kind covers "collect prose" where it is the
  point; a side-thread on every poll is surface area waiting for a use
  (Rallly has them; Doodle dropped theirs).
- **Condorcet, STV, score-each-option, dot-voting, quorum, weighted
  votes.** OpaVote and Loomio exist. IRV+Borda over stored rankings
  covers the consumer bar, and ballots-not-counters means a new tally is
  a pure function later, not a schema change.
- **Per-IP dedup and CAPTCHA.** Weak, painful on exactly our networks
  (a campus NAT makes the class one voter), and the `link` mode's
  honesty-plus-cap is a better posture than pretending either works.
- **Recurring polls.** A trigger can create a poll on a schedule the day
  someone wants it; the poll store does not need its own cron.

## 10. Build order

Each step lands and tests without the ones after it:

1. **Manifest types + tallies** — `PollQuestion` (kind-tagged,
   validated, `times` variant wrapping today's fields), the questions
   list with per-question ids, ballot validation per kind,
   `tally_choice`/`tally_likert`/`tally_vas`/`tally_ranking` pure and
   unit-tested (IRV property tests: majority winner, elimination order,
   exhausted ballots; Likert median on even splits; VAS deciles;
   small-n suppression).
2. **The box generalises** — the `question` column becomes the questions
   list, defaulted on load; POST validation per kind; server-rendered
   components for `choice` and `likert` (the two that are pure
   radio/checkbox forms), JS-off complete. Existing times polls
   indistinguishable before/after: the proof the default is right.
3. **Visibility enforcement** — the two enums on the row, `render` and a
   new `results.json` both consulting (policy, viewer, state); tests
   that a `creator` poll's page carries no counts in its bytes, an
   `anonymous` drain carries no names, a below-floor cell is suppressed.
4. **The reactive layer** — `poll.js` grows the results refresher on the
   `slots.json` pattern; bars animate; `after_vote` reveal on the first
   allowed fetch.
5. **`vas`, `ranking`, `text` components** — the thumbless slider and
   its number-input degraded path, the rank-select fallback and reorder
   enhancement, the textarea with counter. The `text` drain path marked
   `free_text` end to end, with a test that a text answer never reaches
   `status --json`'s typed section. Gallery pages per kind, with the
   exhaustive-match guard so a seventh kind cannot ship unrendered.
6. **Audiences at scale** — `--roster students.csv` minting + `links.csv`
   out; the `link` mode (cookie capability, forced anonymity, required
   `max_ballots`, rate-limit coverage). The course survey ships here.
7. **CLI + home** — `polls create --spec`, `--holds` for times,
   `status --json` emitting per-question tallies beside policy-filtered
   ballots, `close --resolution`, `export --csv` with the injection
   hardening and its test.
8. **The classroom** — the per-poll screen view, its 2s refresh, the
   join URL rendered large. The first lecture runs here.
9. **The TUI `/polls` monitor** — list, detail tallies, response counts,
   close/export as child processes; the lecture controller. Late on
   purpose: every verb it drives must already exist.
10. **The PowerPoint content add-in** — a static manifest XML plus one
    Office.js wrapper page the box serves beside the poll pages
    (SLIDES-RESEARCH.md §3 is the shape: per-insertion screen URL via
    Office settings, edit-view placeholder / show-view live iframe,
    fail-soft when `ActiveViewChanged` doesn't fire). Last on purpose,
    and gated on experience: it embeds the step-8 screen view, so it
    ships only after that view has run a real lecture from a browser
    window — the add-in is a convenience over a proven page, never the
    thing the first lecture depends on. No mecha-core change, no new
    crate; the box grows two static routes.

## 11. Decided along the way

Recorded so they are not re-litigated: **`after_vote` is the default
`show`** (vote, then the summary — the user's call; `times` stays
`live`). **`polls export --csv` ships** (§7, with the injection
hardening). **The TUI `/polls` monitor ships** (§7 — list, tallies,
response counts; it is also the lecture controller). **There is no
series / stable class URL** — the user's call: each poll is its own
link, the lecture deck is prepared as separate polls, and the QR
generator is a future presenter-side CLI nicety (§3.1), not page
machinery. **The PowerPoint content add-in is in** (2026-08-09,
reversing this document's original out-of-scope line): the
SLIDES-RESEARCH.md survey showed it is a manifest plus a wrapper page
over the existing screen URL rather than an app, and the user presents
from PowerPoint and Keynote — it is §10 step 10, after the screen view
has run a real lecture. Keynote stays no-build (no add-in model
exists; browser window or the OBS pipe). The open-link audience went from open question to designed
the day the course case arrived (§3); Likert batteries went from open
question to core when the poll became a list of questions (§1).
**Word clouds are in** (2026-08-09, reversing the research's leave-out):
text answers visualise as a per-ballot-counted, two-ballot-minimum,
stopword-filtered weighted list — on the results page above the listed
answers, and on the projector *instead of* them (§3.1).

## 12. Open questions

1. **The small-n suppression floor.** n < 3 per cell is a starting
   value, not a studied one; course-eval systems use 5. Decide before
   the first anonymous poll with fewer than a dozen respondents.
2. **Screen transport.** The projector view refreshes by 2s polling —
   the `slots.json` pattern at lecture cadence, one client, no new
   machinery. If a hall ever makes that feel laggy, SSE is the upgrade
   path; not before.
