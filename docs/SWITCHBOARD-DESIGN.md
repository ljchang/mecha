# The hangar and the switchboard — design

Decisions, not evidence. `docs/PUBLIC-SURFACE-DESIGN.md` is the parent
document and this is a section of it that grew too large to sit inside §14;
where the two disagree, this file wins for the personal public surface and the
parent keeps everything else. Settled 2026-08-10.

**Still unbuilt.** This is the shape to build, written so someone can start.

---

## 0. What this is for

A person's artifacts are scattered across four namespaces and there is no page
anywhere that says what they have. A collaborator who wants last week's report,
a prospective student who wants to know how to apply, and a colleague who wants
twenty minutes all arrive through different URLs that somebody had to send them
by hand.

Two surfaces close that, and they are the same object wired two ways:

- **The hangar** — `/@{handle}` — everything the user has published and left
  public, grouped, generated. Nobody curates it; it is what the inventory says.
- **A switchboard** — `/@{handle}/{slug}` — a hand-patched set of lines, each
  one going to something that already exists: book a meeting, request a letter,
  apply to the lab, read the teaching material. The page a person puts in an
  email signature.

The names are doing real work. **A hangar shows what you have built; a
switchboard patches someone through to the thing they need.** That is the
difference between the two wirings, and it is why one word could not carry
both — every candidate that fit the display half ("armory", "inventory",
"depot") faced inward and read wrong on a page a stranger uses, and every
candidate that fit the reception half ("desk", "lobby", "kiosk") said nothing
about an inventory.

**The critical property of both: neither adds an inbound path.** Every line on
a switchboard goes to a form, a booking page, a poll or a bundle that already
exists and is already served. The whole chain behind a line — typed submission,
email verification, the queue, `drain`, mecha's `frontdoor.rs` quarantine,
extraction, triage, staged drafts in the outbox — is built and tested. This is
a renderer over settled machinery, and if it ever stops being that, something
has gone wrong.

---

## 1. Decisions taken

| | |
|---|---|
| **Structure** | One type, two kinds — `Hangar` (auto-wired) and `Switchboard` (hand-patched) |
| **Location** | The gate, `/@{handle}` and `/@{handle}/{slug}`. The artifact origin's root 302s here |
| **Authoring** | TOML at home *and* browser edits, three-way merge against a stored baseline, TOML wins conflicts, `pull` closes the loop |
| **In place** | Boards are created and edited in the cockpit as TOML source in a textarea, against the same validator the push endpoint runs (§5.4) |
| **Listing** | A public bundle is listed by default; `--unlisted` and a cockpit toggle opt out |
| **Chrome** | Every surface wears the standard shell; a bundle line goes through `/view/…`, never straight to the artifact origin (§6.1, §6.2) |
| **Theming** | Shipped themes plus an accent. Custom CSS deferred, and scoped to content rather than the shell when it arrives (§6.3) |
| **Entries** | References (`kind` + `id`), resolved server-side. `link` is the honest external escape hatch, rendered with its host visible |
| **Dangling** | Omitted from the page, and reported to the owner |
| **Naming** | The system names the *kinds*; the user names each switchboard |

---

## 2. One inventory, three renderings

Today `/account` answers "what have I got" with `bundles_overview`
(`http/account.rs:164`), which knows about **one** of the four kinds of thing a
user can have. It renders two tables — Artifacts and Machines — and never asks
about `types`, `polls`, `bookings`, `shares` or `view_caps`, all of which are
per-user tables in the same database. So the owner's own view is already
incomplete, before any public page exists.

Both new surfaces need the complete answer, so all three take it from one
place:

```
                    ┌── inventory(user) ──┐
                    │  bundles            │
                    │  forms              │
                    │  booking pages      │
                    │  polls              │
                    └──────────┬──────────┘
             ┌─────────────────┼──────────────────┐
        /account            /@luke          /@luke/teaching
     everything, with      public and       hand-picked,
     controls              listed           ordered, with prose
```

**This is the rule `http/booking.rs:44` already states for a different
computation** — one `open_slots` feeds the page, the race-loser re-render and
`slots.json`, "because two copies of a subtraction is how two surfaces end up
disagreeing about what is open." An inventory that the cockpit computes one way
and the hangar computes another is that bug wearing a new hat, and its failure
mode is *my homepage advertises a form I deleted*.

The three renderings differ in a **filter** and a **chrome**, never in a
source. That is what makes "the public page can only ever show less than the
private page" true by construction rather than by review.

**Fixing `/account` is therefore step one**, and it is worth doing on its own
account. It produces the function the other two consume.

### 2.1 What counts as an artifact, precisely

Four kinds, because that is what the schema actually has:

- **Bundles** — `bundles` joined to `aliases`. Public iff the alias says
  `public` *and* names a version. A bundle published but never aliased is not
  yet a publication (`http/artifacts.rs:73`), and a withheld one is nothing.
- **Forms** — rows in `types` whose manifest parses to
  `RequestKind::Request`. Served at `/f/{handle}/{id}`.
- **Booking pages** — rows in `types` whose manifest parses to
  `RequestKind::Booking`. Served at `/s/{handle}/{id}`.
- **Polls** — rows in `polls`. **Listable only in link mode and only while
  open.** A poll whose audience is per-participant capability tokens has no URL
  to link to — every real URL contains somebody's ballot capability — so
  listing it is not merely useless, it is the shape of an accident. And a
  closed poll is a dead button.

Surveys are polls with a `spec`, not a fifth kind. Slides are the PowerPoint
add-in (`/slides/addin`), not a per-user artifact. `shares`, `viewer_links` and
`view_caps` are permission grants rather than artifacts: they belong on the
cockpit — *who can currently read what* is exactly the question an owner needs
answered — and they never appear on a public surface, since the audience for a
private grant is one person who already has the link.

---

## 3. The hangar

### 3.1 Where it lives, and why not the artifact origin

**On the gate**, at `/@{handle}`. Three reasons, in increasing order of how
much they cost to get wrong:

1. Every other per-user interactive surface is already there — `/f/…`,
   `/s/…`, `/p/…`, `/signup/…`, `/account`. A page that links exclusively to
   another origin is on the wrong one.
2. The artifact origin's entire caching story is two rules: version URLs are
   immutable forever, the alias is never cached. A live index that changes
   whenever anything is published is a third rule on an origin with two.
3. **The artifact origin serves user-authored bytes.** The hangar is *our*
   HTML about a user. Keeping them on separate origins means a published
   bundle's JavaScript is never same-origin with a page we render.

`{handle}.{artifacts-origin}/` currently serves nothing. It should **302 to
`/@{handle}`**, so both URLs work and there is one implementation. A person who
has only ever seen an artifact URL will try its root, and getting nothing is a
worse answer than a redirect.

This raises the stakes on §13.1 of the parent document (*which domains*).
`/@luke/teaching` is going into an email signature; the gate's name stopped
being configuration and became a product decision.

### 3.2 A profile is data, never a page

**There is no field that accepts HTML, markdown, or CSS.** Typed fields,
escaped at render, theme chosen from a validated set.

`mecha-manifest/src/theme.rs` already makes this argument for forms and it
transfers without modification: *"a model that writes CSS per request produces
a different form every time, which is the opposite of a surface people learn to
trust… A theme is tokens, never rules."* If a theme could add selectors it
becomes where layout fixes go — *just this one page needs the heading bigger* —
and within a month no two pages render alike.

The second reason is narrower and sharper: the gate is where the `__Host-`
session cookie lives. `http/account.rs:21` already reasons carefully about a
tenant's page being unable to toss a cookie onto the gate; author-controlled
markup rendered *by* the gate would walk straight through that.

The shape of the record:

```toml
enabled      = true                 # default false
display_name = "Luke Chang"
tagline      = "Computational social neuroscience · Dartmouth"
bio          = "Two or three sentences. Plain text."
location     = "Hanover, NH"
timezone     = "America/New_York"   # rendered beside a booking line
theme        = "slate"
accent       = "#5d5294"            # optional, validated

[[link]]
kind = "github"
url  = "https://github.com/ljchang"

[[link]]
kind  = "website"
label = "Computational Social Affective Neuroscience Lab"
url   = "https://cosanlab.com"
```

`kind` is drawn from a known set (`website`, `github`, `scholar`, `orcid`,
`mastodon`, `bluesky`, `linkedin`) which earns a **shipped inline SVG**;
anything else renders with its hostname visible and no icon. The icons must be
inline — an `<img src="https://…">` is precisely what the publish gate fails a
bundle for, and the CSP would block it anyway.

`enabled` governs the **hangar only**. A switchboard whose URL is in somebody's
email signature keeps working when the hangar is switched off, because those
are separate publications with separate audiences and conflating them would
make "hide my index" silently break a link a stranger is holding.

**The avatar is the one field with teeth.** It cannot be a URL, so it is an
uploaded blob and it pulls in the `attachments` path, a content-type sniff, and
a size cap. It is legitimately a second-version field; the record renders
without it.

### 3.3 Listed is not public, and the default is listed

These are two questions and the schema currently has one. A report released
publicly so that a single collaborator could open it is *public and unlisted* —
reachable by its URL, not advertised on a front page.

**A public bundle is listed by default.** `public` was already the deliberate
act, and an empty hangar reads as broken and never gets used. Opting out is
`--unlisted` at publish and a toggle in the cockpit.

The invariant underneath, which is the one worth stating as an invariant:

> **The hangar is a view, never a permission.** It can only ever subtract from
> what visibility already allows.

Which means the listing pass must reach its conclusions through the *same*
`access()` in `http/artifacts.rs:73` that serving does, not through its own
copy of the rules. A second opinion about who may read what is how a private
bundle ends up named on a public page — the title alone is a leak, even if the
bytes stay refused.

### 3.4 Grouping

**Not by bundle `class`.** `static` / `interactive` / `compute` is a CSP axis
and no reader has ever cared about it.

Group by what the thing is *for*, derived from kind by default and overridable
per artifact — *Reports · Notebooks · Talk to me · Polls · Teaching*. The
default derivation is what makes a hangar useful the day it is enabled; the
override is what lets it be organised the way its owner actually thinks. The
group is a property of the artifact, which matters for §5: it lives on the
artifact's own row, has exactly one writer, and never enters the merge.

Order within a group: pinned first, then recency.

---

## 4. The switchboard

### 4.1 An entry is a reference, not a URL

```toml
# ~/.mecha/factory/switchboards/hello.toml
slug    = "hello"
heading = "Get in touch"
intro   = "The fastest way to reach me is one of these."

[[entry]]
kind  = "booking"
id    = "office-hours"
label = "Book a meeting"
blurb = "20 or 45 minutes, usually free within a week."

[[entry]]
kind  = "form"
id    = "recommendation"
label = "Request a letter"
blurb = "I'll need three weeks and your CV."

[[entry]]
kind  = "bundle"
id    = "teaching"
label = "Teaching resources"

[[entry]]
kind  = "link"
url   = "https://cosanlab.com"
label = "Lab website"
```

**`kind` + `id`, resolved against our own tables** — not a URL string. This is
the same move `RequestType::servable()` and `booking.rs:98`'s `resolve` already
make, and it buys two things a string cannot:

- The server can **prove** the target exists, belongs to this user, and is
  public. A URL can only be trusted.
- An entry cannot quietly point at another tenant's artifact while wearing this
  user's handle and theme.

`kind = "link"` is the honest escape hatch, because a real person's page needs
to link to their lab site. It renders **with its hostname visible**, which is
the same distinction the publish gate already draws between `<a href>`
(navigation, never a finding) and `<img src>` (the page reaching out, always
one). A page that made an off-origin link indistinguishable from a first-party
one would be a phishing kit with a nice theme.

### 4.2 A dangling entry is omitted, and the owner is told

A form gets deleted, a bundle is taken down, a poll closes. The entry is
**removed from the rendered page**, and the fact is surfaced in the cockpit.

Fail-closed in the direction that matters: a dead button on the page in
someone's email signature is worse than an absent one, because the person who
clicks it concludes something about you rather than about the software. This is
also exactly the kind of thing a nightly trigger should stage a warning about
— *three lines on `hello` are dark* — rather than something the owner has to
remember to check.

### 4.3 The user names the board, and that is what makes the naming work

`slug` and `heading` are the user's words. A visitor sees **Teaching**, or
**Get in touch** — never "switchboard".

That is not a cosmetic detail; it is what made the naming tractable at all.
Every objection to a mecha-native word was really an objection to showing it to
a stranger. Once each board carries its own name, the system word only has to
be legible to the owner and to the code, which is a far lower bar — and it is
why "hangar" survives despite a hangar being somewhere you would never send a
prospective student.

The precedent for one type with two user-facing names is already in the tree:
`RequestType` carries `RequestKind::{Request, Booking}` (`request.rs:112`), the
world says "a form" and "a booking page", and nobody has ever said "request
type" out loud. So:

```rust
enum BoardKind { Hangar, Switchboard }
```

rather than `Generated | Declared`. The second pair describes how the entries
got there; the first describes what the page is for, which is what a reader of
*the visibility filter must run on both kinds* actually needs to know.

---

## 5. Authoring: two writers, one record

The record can be edited from home (TOML, pushed) and from the browser
(the cockpit). **TOML wins conflicts** — but that has to mean something
sharper than "the last push clobbers", or it has an ugly failure: fix a tagline
on your phone, the nightly push runs from an unchanged local file, and the fix
silently reverts. That is the silently-degrading shape this project keeps
naming.

**First, shrink the problem.** Most of what anyone edits in a browser is not
profile-record data at all: `listed`/`unlisted`, which group an artifact sits
in, whether a board is live. Those are properties of *the artifact*, they live
on the artifact's row, and they travel the same single-writer path
`alias_set` already defines. They never enter the merge. What overlaps is
small — display name, tagline, bio, location, links, theme, and a board's
entries.

### 5.1 The three-way merge

```
  the box stores:  baseline   the TOML exactly as last received
                   effective  what the page renders

  push(new):       changed = diff(new, baseline)
                   apply changed over effective     ← TOML wins collisions
                   baseline = new
                   report every field it overwrote
```

No timestamps and no per-field provenance: the baseline is sufficient. "TOML
wins" becomes "TOML wins *conflicts*", which is what makes a browser edit worth
making. Roughly thirty lines.

The cheaper fallback, if this proves fiddly in practice, is **disjoint
ownership** — TOML owns identity, links and theme; the browser owns only
toggles — which needs no merge at all. It is a smaller feature, not a simpler
version of this one.

### 5.2 `pull`, or home drifts forever

`factory-publish profile pull` writes the effective record back to
`profile.toml`. Without it every browser edit is a future casualty, and the
cycle never closes.

The cockpit says so plainly rather than leaving it to be discovered: *two
fields edited here are not in your pushed file — run `profile pull` to keep
them.*

### 5.3 Why this disagrees with §14.7, on purpose

§14.7 of the parent document settled the same question for **handlers** the
other way round: "the signed copy on the box is what is live, because that is
what a different machine would attach to."

That is right for a handler and wrong here, and the distinction is worth
writing down because someone will otherwise apply it by analogy and get it
backwards:

> **The box is authoritative for what other machines must agree on. Home is
> authoritative for what a person wrote.**

A handler is behaviour another agent picks up, so the box is the only place two
machines can agree on it. A profile is content, and its author is a person
sitting at one particular machine.

### 5.4 Creating and editing a switchboard in the cockpit

A board can be **created and edited in place** from the cockpit, as its source
text in a textarea. So the same board can be authored by the agent (write the
file, push) or by a person (open the cockpit, type), and neither is the
special case.

**The source is TOML**, the format everything else here already speaks —
`mecha.toml`, the trigger store, the request types, `RequestType::from_toml`.
One stored format is what keeps §5.1's baseline unambiguous and `pull` certain
of what to write. It also happens to be the right format for a *textarea*
specifically: TOML is line-oriented, so a bad line is a bad line and the error
can name it, where an indentation-sensitive format in a box with no auto-indent
and no tab handling is at its worst.

Four rules the editor has to hold:

- **The validator is `mecha-manifest`'s, and it is the same one the push
  endpoint runs.** One validator, two doors — the property parent §4 already
  claims for request types. A board that the cockpit accepts and a push
  rejects, or the reverse, is the bug this rules out.
- **A rejected save never loses the text.** The page re-renders with the
  submitted source still in the textarea and the error pointing at a line.
  Losing forty lines of typing to a misplaced bracket is how an in-place
  editor stops being used, and `mutating()` as written re-renders a *stale*
  page — this is the one place that has to preserve the body.
- **A dangling reference warns, it does not refuse.** Writing the board before
  creating the form it points at is an ordinary order to work in. Save
  succeeds, and the page says which lines are dark — which is §4.2's report
  arriving at the moment it is most useful.
- **Creating a board claims a slug, and §8 says that is permanent.** So the
  create form states it and confirms rather than minting a name as a side
  effect of typing one. A "New switchboard" button that quietly burns a name
  that can never be reissued is a trap, and it is exactly the kind of trap
  that only shows up on the second attempt at a name.

**This does not give a session the ability to publish**, and the distinction
matters because it looks at a glance as though it does. §7's rule is about
*bytes*: a browser session cannot push a version, and every artifact a line
points at was published by a scoped key. A board is a list of references to
things that already exist and are already permissioned — authoring one grants
no read that did not already exist, which is §3.3's invariant doing its second
job.

**And `pull` gets more load-bearing**, because a board may now be created
somewhere home has never seen. `factory-publish switchboard pull` must be able
to *create* local files, not only update them, and `switchboard list` is what
tells a person what exists remotely.

---

## 6. One shell, the viewer for everything, and the door left open for CSS

### 6.1 Every surface wears the standard chrome

**A hangar and a switchboard render through the same `shell_with` and the same
`Chrome` as every other gate page.** One header, one mark, one stylesheet, one
set of controls — the reader moves from a board to a form to a report without
the ground shifting under them.

An earlier draft of this section said the opposite — that a public board should
carry *no* chrome, on the grounds that a page in an email signature should not
advertise the factory, and that a page with no chrome gives future custom CSS
nothing to spoof. **`http/viewer.rs` already makes the better argument**, and it
is worth quoting because it is the reasoning that governs here too:

> *rather than carrying identity out to the artifact origins — where every
> tenant's published scripts run, and where an ambient credential would be
> theirs to spend — the viewer lives here, where the session already is, and
> frames the bundle cross-origin. The gate page is ours, so it may know your
> handle, wear the account dropdown, and hold the release controls.*

The chrome is safe precisely because it is **ours, on our origin, wrapped
around content that is framed rather than inlined**. Removing it buys nothing
that isolation is not already buying, and costs the consistency that makes a
set of pages feel like one system.

The narrow half of the old objection survives and is already expressed in the
type: `Chrome::Public { sign_in }` is parameterised exactly so that *"the
splash, not a stranger's form"* gets the sign-in box
(`http/intake.rs:56`). Boards take the same answer a stranger's form takes.

```
  stranger        Chrome::Public { sign_in: false }   mark · Docs
  owner, signed   Chrome::Account { .. }              mark · dropdown ·
                                                      "edit this board"
```

The owner seeing their own board signed in gets an edit affordance in place,
which is where someone actually notices a line is wrong.

### 6.2 A bundle line goes through the viewer, never straight to the origin

When a hangar or a switchboard names a bundle, the link is the **two-segment**
`/view/{handle}/{id}` — not the artifact origin's share URL, and not the
version-pinned `/view/{handle}/{id}/{version}`.

The two-segment spelling (`viewer::two_seg`, `http/viewer.rs:361`) follows the
alias, which is what a line must do: a board that pinned a version would go
stale the next time its owner published, and silently — the page would keep
working while showing last month's report. Same reasoning as the share URL
never being cached.

Verified while checking this design against source: `viewer.rs:529` allows a
public, aliased bundle on `(public && live.is_some())` alone, with no session —
so an anonymous stranger following a line from a switchboard gets the framed
view, chrome and all, with nothing to sign in to.

That single choice buys most of the consistency:

- The reader keeps the header, the styling and the controls on every artifact,
  instead of being dropped onto a bare origin serving raw bytes.
- The owner viewing their own artifact gets the release controls right there,
  on the same CSRF machinery the cockpit uses.
- A **private** bundle behaves correctly with no extra work: the viewer
  already decides who may read, mints a short-lived capability, and frames
  `/g/<cap>/`. A hangar never lists a private bundle (§3.3), but a
  switchboard's owner may well patch a line to one they have shared, and that
  path already exists.
- The artifact origin keeps its two rules (§3.1) and learns nothing about
  identity.

The canonical artifact URL still exists and is still the thing to send someone
who wants raw bytes; the viewer is the framed reading experience, and it is
what a *line* points at.

### 6.3 Theming, and what custom CSS will have to respect

**Now:** a `theme` field over the validated set in `mecha-manifest/src/theme.rs`,
plus an optional accent. Nothing else.

**Later, wanted:** custom CSS. With §6.1 settled, the preparation is no longer
"leave the chrome off" — it is the rule `theme.rs` already enforces, applied
one level up:

> **A theme, and later a stylesheet, may style the page's content. It may
> never reach the shell.**

That is the same shape as *tokens, never rules*: the structural sheet and the
header are shared and are not addressable from a board's own styling. Which
means the spoofing objection is answered by scoping rather than by absence —
and answered better, because the consistency survives.

Two more things keep the deferral honest:

- **Class names are not stable and are not promised to be.** Custom CSS against
  churning markup breaks every release. When CSS ships, a documented and frozen
  set of hooks ships as part of the same decision, not afterwards.
- **The escape hatch already exists.** Anyone who wants total visual control
  today can publish an `interactive` bundle and point a switchboard line at it.
  Full control, already vendored, already through the external-reference gate.
  Saying so out loud takes the pressure off shipping CSS early.

---

## 7. The cockpit

`/account` is the owner's operational surface and the word for it is
**cockpit** — the seat the pilot sits in, next to the hangar the machine stands
in. It is a better heading than the current "Artifacts / Machines", and the
rename costs nothing because that page needs the inventory rewrite anyway.

What it holds, after §2: all four artifact kinds rather than one; the
listed/unlisted and grouping toggles; which version each alias points at; the
machines and their keys; live shares and view grants (*who can currently read
what*); dark lines on any board; the drift notice from §5.2; and — per §5.4 —
the board editor and the create form.

What it still cannot do, unchanged from `http/account.rs:37`: **publish.** A
browser session that could push bytes would be a third write path with none of
the review shape. Authoring a board is not an exception to that; it writes
references, never bytes (§5.4).

---

## 8. A slug is permanent, like a handle

`/@{handle}/{slug}` is a path namespace, and a slug goes into an email
signature. It can never be taken back — the same argument that makes a handle
unreusable (`db.rs:2951`): every URL somebody put in a paper resolves to
whatever the name means next.

Two consequences:

- **Reserve the short and structural names before the first slug exists.**
  `v`, `b`, `f`, `s`, `p`, `g`, `a`, `account`, `signin`, `signup`, `admin`,
  `view`, `slides`. Retrofitting a reserved list means either breaking a live
  URL or never using that route.
- **A retired slug resolves to nothing**, and is never reissued to a different
  board — for the reason a retired handle serves nothing rather than serving
  whoever claims it next.

---

## 9. What the security model does not change

Worth stating explicitly, because a new public surface invites the assumption
that something moved:

- **No new inbound path.** Every line goes to an artifact that is already
  served. Nothing here accepts a submission.
- **No new taint source.** A board renders the owner's own words and the
  server's own data. There is no stranger text anywhere on it, so nothing here
  reaches `frontdoor.rs`.
- **No new trust in the box.** The record is content, and a compromised box
  could already serve wrong bytes for a bundle. Nothing about a board needs
  signing that a bundle did not.
- **No new origin, and no CSP change.** Gate policy, unchanged.

The one genuinely new thing is a typed record with two writers, which is §5.

---

## 10. Deliberately not in this

Recorded so the argument is not had twice.

- **A directory of users.** There is no `/@` that lists everybody, and no
  search across tenants. That would make the factory a social network, and
  parent §15.3 already committed us to being responsible for what tenants
  publish — a discovery surface multiplies that obligation into moderating who
  may be *found*, which is a different and much larger thing to take on. A
  hangar is reached because its owner gave you the URL.
- **Anything social on the page.** No comments, no guestbook, no reactions, no
  follower count. Each is an inbound path with a moderation story attached, and
  §0's whole claim is that this surface adds no inbound path.
- **Editing artifacts from a board.** It is an index, not a CMS. Publishing
  stays the machines' job through scoped keys (§7).
- **Markdown or HTML in `bio` and `blurb`.** §3.2, and the request will arrive
  phrased as "just links, surely".
- **A feed.** RSS or JSON, per user. Reasonable, cheap, and genuinely useful
  for a collaborator who wants to know when a report lands — but it is a second
  serialisation of the inventory with its own caching and visibility rules, and
  it should be designed rather than added.
- **Aggregate stats on the page.** "412 views", "published 37 artifacts". It
  invites a metric to be gamed and tells a stranger something about the owner's
  week that they did not choose to say — the same instinct as parent §14.9.4.

---

## 11. Build order

1. **`inventory(user)`, and the cockpit rendering all four kinds.** The query
   the other two surfaces consume; independently worth doing, because
   `/account` is wrong today.
2. **The record in `mecha-manifest`**, `PUT /v1/profile`, `factory-publish
   profile push` / `pull`, the merge, and the MCP tool. No page yet.
3. **`GET /@{handle}`** — the hangar. Generated lines, grouping, the `enabled`
   toggle, the artifact-origin redirect, the reserved slug list. Standard
   shell, and bundle lines pointed at `/view/…` (§6.1, §6.2).
4. **`GET /@{handle}/{slug}`** — switchboards. Declared lines over the same
   renderer, dangling-line omission, the cockpit's dark-line report.
5. **The cockpit editor** (§5.4) — create with slug confirmation, edit as
   TOML source, validate without losing the text, warn on dark lines. Last of
   the core steps on purpose: it is the one that needs the record, the
   validator, the merge and a rendered page to already exist.
6. Avatar.
7. Per-line counts, if §12.3 is answered yes.

---

## 12. Open decisions

1. **The gate domain** (parent §13.1, still open). No longer only
   configuration — see §3.1.
2. **Avatar in the first version, or the second.** It is the only field that
   pulls in the blob path.
3. **Per-line click counts.** Wanted — *does the recommendation line actually
   get used* is a real question — but it is a new data class (visitor
   behaviour) on a box we have agreed to assume is lost, and it needs its own
   answer about what is stored and for how long, under parent §15.4. Ship
   without it; add it as opt-in, no cookie, no IP.
4. **Custom domains** (`luke.example.com` → a hangar). Pure ACME and config
   work, which is the kind that reads as easy and is not.
5. **Whether `enabled` defaults false forever.** It does for now, which is the
   right default for an existing user who did not ask for a public page. Once
   signup mints accounts that never had artifacts anywhere else, "off by
   default" may be the wrong greeting.
6. **Whether a board has a draft state.** Created-then-edited means a board is
   briefly live and half-written. Nobody has the URL yet, so the exposure is
   theoretical — but `live = false` until the owner says otherwise is cheap,
   and the alternative is discovering the answer from someone's email
   signature.
