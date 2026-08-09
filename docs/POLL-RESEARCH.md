# Polling — what the landscape covers

*2026-08-08. The survey behind POLL-DESIGN.md: what polling products ship,
so the general-purpose poll generalises from evidence rather than from the
one scheduling case it grew out of.*

## 1. The categories

**Quick single-question polls.** StrawPoll is the archetype: one question, a
handful of options, share a link, no accounts for voters. Twitter/X polls
(≤4 options, 5 minutes–7 days) and Reddit polls (2–6 options, 1–7 days) are
the embedded-in-a-feed variant. Slack has no native polls — the folk pattern
is emoji-reaction voting, and the real products are marketplace apps (Simple
Poll: single-choice free, anonymous and multi-choice paid; Polly). Discord
shipped native polls in 2024: ≤10 options, optional multi-select, fixed
durations, and — notably — **never anonymous**: anyone can click "View
votes."

**Scheduling/availability polls.** Doodle is the canonical grid: rows =
people, columns = slots, cells = yes/no/if-need-be. When2meet is the
minimalist free cousin — paint availability, watch the overlap heatmap
darken live, no accounts. Rallly is the open-source self-hostable Doodle
(tri-state voting, no-signup participation, timezone conversion, real-time
updates, comments). Framadate (Framasoft) does both date polls and "classic"
subject polls. These are a *distinct shape*: the vote is per-(person,
option) cell, not pick-one-of-N — which is exactly what mecha's seeded group
poll already is.

**Form/survey builders.** Google Forms and Typeform can express any poll,
but "survey" exceeds "poll" on several axes: multi-page, conditional
branching, quiz grading, per-respondent piping. A poll is one question with
shared, comparable results; branching means you are building a survey
engine. (mecha-factory already has one: a multi-step request form *is* the
survey shape. What a form lacks and a poll has is results served back to
the participants.)

**Decision / ranked-choice tools.** OpaVote is the serious end: real
elections, ranked ballots, IRV, ~9 STV variants, several Condorcet
variants, Borda, Approval — with voter lists by single-use email code.
RankedVote is the lightweight consumer version: IRV only, with a
round-by-round elimination visualization. StrawPoll also ships a
ranked-choice type (IRV tally). The design lesson: **ballot format and
tally method are separable** — store the ranking, compute IRV and Borda
from the same ballots.

**Live audience response.** Mentimeter (13 question types), Slido, Poll
Everywhere. Defining traits: presenter screen with live-animating charts, a
join code, votes usually anonymous, results revealed at the presenter's
discretion. Mostly *presentation choreography*, not poll semantics.

**Community/consensus voting.** Loomio treats a poll as one step in a
*decision*: proposal types (advice/consent/consensus), poll types Choose,
Score (rate each option), Rank, Dot-vote (spend a point budget), Time poll
— each with a discussion thread and an **outcome statement** attached to
the close. The outcome statement is worth stealing; the governance
machinery is not.

## 2. Question / response types

| Type | Who ships it | Notes |
|---|---|---|
| Single choice | everyone | the default everywhere |
| Multiple choice | StrawPoll, Discord, Slack apps, Mentimeter | StrawPoll and Simple Poll cap selections ("choose up to N"); Discord's is uncapped |
| Ranked choice | OpaVote (everything), RankedVote (IRV), StrawPoll (IRV), Loomio, Mentimeter | consumer products tally **IRV** almost universally; Condorcet is effectively OpaVote-only |
| Rating / scale | Loomio Score, Mentimeter, Slido, survey tools | two variants: rate *the subject* (satisfaction) vs rate *each option* (score voting) |
| Yes/No/If-need-be | Doodle, Rallly, Framadate | the tri-state cell is the whole point of availability polls; When2meet is two-state |
| Voter-added options | StrawPoll, Framadate | powerful for "where should we eat"; needs dedup/moderation — and on our surface, a quarantine story |
| Image options | StrawPoll, Mentimeter | option = text + image; cheap in a schema |
| Word cloud / open text | Mentimeter, Slido | a live-event feature, not a poll |

## 3. Visibility and anonymity — two independent axes

Products conflate these; they are separable and both matter.

**When a voter sees aggregate results:**

1. **Live/always** — Doodle, Rallly, When2meet (the grid *is* the results),
   Discord, Slack apps.
2. **After you vote** — Twitter/X, Reddit, a StrawPoll setting. Prevents
   bandwagon/anchoring; a Framadate issue explicitly calls always-visible
   results "unfair to early pollers."
3. **After close** — StrawPoll setting, OpaVote, Mentimeter/Slido
   (presenter reveals).
4. **Creator only** — StrawPoll "keep hidden", Doodle hidden polls.

**Who can see who voted for what:**

1. **Named grid, visible to all** — Doodle default, When2meet, Rallly,
   Framadate, Discord. The norm for scheduling: *that Alice can only do
   Tuesday* is the data.
2. **Names visible to creator only** — Doodle's "hidden poll": participants
   see only their own response; the organizer sees everything.
3. **Aggregate only, creator included** — Twitter/X (not even the author
   sees identities), Mentimeter/Slido anonymous mode, Simple Poll/Polly
   anonymous polls.

Defaults by genre: scheduling tools default to the public grid; quick-poll
tools default to anonymous-with-visible-counts; social platforms hard-code
one model each. The trust asymmetry between models 2 and 3 is real —
"anonymous to peers" and "anonymous to the creator" are different promises,
and the page must say which one it is making *before* the vote.

## 4. Integrity / duplicate prevention

The ladder, weakest to strongest: none → browser cookie (incognito evades
it) → per IP (breaks on lab/campus NAT, evaded by proxies — a cottage
industry of StrawPoll proxy bots exists, which says what link-only polls
are worth) → per email / single-use code (StrawPoll Pro invitations,
OpaVote's model — the strongest without an account system) → authenticated
account. **mecha's per-participant capability URL is already the
email-code rung**, the strongest practical one.

**Vote editing:** scheduling tools allow it (Framadate makes it per-poll
policy); X and Reddit do not; Discord allows it while open. Editing
interacts with anonymity: changing *your* vote requires the server to know
which vote is yours, even if it never shows anyone else — so "anonymous" in
every real product is a serving policy, not cryptographic unlinkability.

**Deadlines:** StrawPoll has them; X/Discord/Reddit have fixed durations;
Doodle sells them as a paid feature.

## 5. Lifecycle and mechanics

- **States:** draft → open → closed everywhere; close by deadline or by
  hand. Social polls close permanently; scheduling tools can reopen.
- **Finalization:** scheduling polls have a terminal step others lack —
  "book it" — and Loomio attaches an outcome statement to any closed
  decision. Generalisation: a closed poll carries a creator-authored
  **resolution**.
- **Quorum:** absent from consumer tools; governance land. Skip.
- **Comments:** Rallly, Framadate, Loomio have per-poll threads; Doodle
  dropped theirs over the years.
- **Notifications:** Framadate optionally mails on every vote; Doodle and
  Rallly notify creators; reminders to non-responders are a paid Doodle
  feature (mecha's design already has the one-nudge rule).
- **Export:** CSV/JSON is table stakes in the self-hosted tier (Framadate,
  OpaVote full ballots).
- **Live results:** Mentimeter/Slido are the benchmark feel — bars animate
  as votes land; When2meet's heatmap darkens live. The feel that matters
  most: **a voter who just voted sees their bar tick up immediately**.
  Note the interaction: live results are exactly what "hidden until
  vote/close" exists to suppress — one enum, not two features.

## 6. What ~90% coverage needs, and what to refuse

Covers real use: four question kinds (choice with min/max selections,
ranking tallied IRV with Borda as a second read of the same ballots, rating
on a declared scale, availability tri-state), options as text + optional
link/image, the two visibility axes as two enums, editable votes while
open, a deadline plus manual close, a resolution on close, cheap
live-refresh of results, JSON export.

Deliberately out: multi-question surveys and branching (the request form is
that product); quorum, weighted and delegated voting (governance software);
Condorcet/STV (OpaVote exists); word clouds, Q&A, presenter mode
(live-event tools); per-IP dedup and CAPTCHA (weak, painful on shared
networks, unnecessary when the audience is a roster); recurring polls; and
open unauthenticated voting at internet scale — the proxy-bot ecosystem
around StrawPoll is the measured cost of that fight.

Sources: strawpoll.com/help/voting-types · support.strawpoll.me (duplication
checking) · github.com/lukevella/rallly · help.doodle.com (hidden polls;
deadlines) · opavote.com/methods/overview · rankedvote.co key features ·
Discord Polls FAQ · help.loomio.com poll types · framadate.org · Framadate
issue #462 · Zapier on Slack polls · Wooclap's Mentimeter/Slido comparison.
