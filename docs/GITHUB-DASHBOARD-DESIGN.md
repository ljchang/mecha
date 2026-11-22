# The GitHub board — design

**2026-11-17.** What is being designed: a board that shows, across the
repositories the owner chooses, what is **failing** (CI on the default
branch), what is **new** (issues, and pull requests from outside
contributors, each with a short summary and a way to open it), and how much
each repository is **used** — fed by a collector that runs with no model
holding a credential, and drawn by the HUD.

**Partly built** — steps 1 (the collector, #894) and 2 (the board, #902) are merged, step 3 (the typed reference, #908) and step 4 (the summaries, #909) are merged, step 5 (usage) is built less conda and container counts (§7.2), step 6 (the picker) is this change (§7.3), and §7 tracks the rest. It extends [`LIVE-DASHBOARD-DESIGN.md`](LIVE-DASHBOARD-DESIGN.md)
and does not restate it; where the two disagree about the HUD, that file wins
until a ruling here amends it, and §0 says which do. It is the first
integration with a service that is neither mail nor documents, and it is
shaped so that it needs neither an MCP server nor the credential vault the
owner wants researched before anything is built (§8).

---

## 0. Rulings

Taken by the owner on 2026-11-17, in the session that produced this file.

| # | Ruling |
|---|---|
| G1 | **The board tracks three things**: CI failures, new issues with a quick summary, and how much each repository is used (downloads or installs where they can be counted). |
| G2 | **An issue opens from the board.** The owner asked for links on HUD boards; §4 is the form proposed, and it amends `LIVE-DASHBOARD-DESIGN.md` §2.2 for one typed case. |
| G3 | **The owner chooses which repositories are included**, from their own account and from their organizations, through a page in the web app. |
| G4 | **Forks are excluded by default.** |
| G5 | **New pull requests from outside contributors appear alongside new issues.** |
| G6 | **The first set is broad** — every non-archived repository the owner owns, and the organizations the owner named. Which organizations is the owner's configuration and lives in the selection file (§2), never in this repository. |

Taken by the owner on 2026-11-18:

| # | Ruling |
|---|---|
| G7 | **G2 is met by the typed reference (§4)**, not by a pane outside the HUD. `LIVE-DASHBOARD-DESIGN.md` §2.2 points here for the one exception. |
| G8 | **Quick summaries come from a quarantined pass in the collector (§5).** |
| G9 | **`gh` holds the GitHub login (§8)** until the credential vault is designed. The vault's research waits until the autonomous-work shape (follow-ups, unattended ask-and-resume) is built, so that its use cases are known. |

Taken by the owner on 2026-11-19:

| # | Ruling |
|---|---|
| G10 | **The collector runs once a day to start** (`scripts/mecha-github-collect.timer`, 06:40 in the owner's zone, finished before the 07:00 `morning` briefing, catching up a missed day). A shorter cadence is a change to that file alone; `collect_log`'s `finished` row records the API quota each run spent, which is what a shorter one would be judged by. |

Nothing is left open (§9).

## 1. The shape

```
systemd timer ──▶ mecha github collect ──▶ ~/.mecha/github/github.db ──▶ HUD loaders ──▶ the board
                    │   (no agent loop)          (sqlite, history kept)      (read-only, R4)
                    ├─ gh api …   the credential stays in gh's own store
                    └─ quarantined pass, one issue at a time ──▶ a typed summary row
```

This is `system.rs`'s shape again, and for the same reason: **the HUD holds
no code for any one source and nothing in it samples**
(`ARCHITECTURE.md` §The HUD). A collector writes a SQLite file; the board
reads it through an ordinary `sqlite` source registered in
`~/.mecha/hud/sources.toml` with `content = "third-party"` written out —
an issue title is a stranger's text. Leaving the line off classifies the
same way (`LIVE-DASHBOARD-DESIGN.md` §3.1, fail-closed), but the file
should say what this design argues, as §2's `forks = false` does.

The collector is a command, never a trigger. A trigger's action is a prompt
(`ARCHITECTURE.md` §Triggers), and nothing here wants an agent: the fetch is
fixed, and the one model call it makes is a quarantined one-shot.

## 2. Which repositories

`~/.mecha/github/repos.toml`, owner-written, never in layered config — a
cloned repository must not add itself to a watch list that spends the
owner's API quota and puts its issue text in front of a model.

```toml
# Owners whose repositories are included unless excluded below.
[[owner]]
login = "example-lab"
include = "all"          # or "listed"
forks = false            # G4; the default, written so the file says it
archived = false

[[repo]]                 # an exception either way
name = "example-lab/old-course-site"
include = false
```

The picker (G3) is a web settings page over this file and the collector's
inventory table: each owner the login can see, its repositories with a
search box, select-all per owner, and a "pushed in the last year" filter.
The page lists from the inventory the collector last wrote; it does not call
GitHub from a request handler, so a slow API is a stale list with its age
shown and never a page that hangs. Writing the file is the owner's act,
through `serve::owner_guard` like every other settings write.

The login used at design time sees about four hundred non-archived
repositories across fifteen owners, most of them not the owner's to watch,
which is why selection is G3 and not a later nicety.

## 3. What is collected

One pass per owner, by GraphQL where one query can cover many repositories,
REST where only REST has the number.

| Table | Rows | From | Kept |
|---|---|---|---|
| `inventory` | every repository the login can see, selected or not | `gh api` | latest |
| `ci_runs` | default-branch workflow runs: workflow, conclusion, started, sha | Actions API | 90 days |
| `items` | open issues, and pull requests whose author is not a member (G5): number, title, author association, created, updated, labels, comment count | GraphQL | while open, then 30 days |
| `summaries` | one row per item per body revision (§5) | quarantined pass | with its item |
| `traffic` | clones and views per day, total and unique | traffic API | **forever** |
| `releases` | download count per release asset, per day sampled | releases API | forever |
| `counts` | stars, forks, watchers, open issues, per day | GraphQL | forever |

Three things about usage (G1), because none of them is the number the owner
asked for and the board must not pretend one is:

- **GitHub keeps traffic for fourteen days** and only shows it to accounts
  with push access. History exists only from the day the collector first
  ran, so the board shows "since <date>" beside any total, and a repository
  the login cannot push to shows a dash — a dash is never zero.
- **A clone is not an install.** CI clones, mirrors and bots are in it. The
  column is named for what it is.
- **Package registries are where installs are counted**, and they need a
  repository-to-package mapping GitHub does not hold: PyPI, conda, npm,
  container pulls. That is step 5 (§7): detect the package name from the
  repository's own metadata, show it in the picker, let the owner correct
  it. "Used by" dependents are on GitHub's web page and not in its API; out
  of scope.

**Refused and failed stay different findings.** A rate limit, an expired
login and a repository that vanished each record a row in a `collect_log`
table naming which, and `mecha doctor` reads it: a board showing yesterday's
green because the collector has been failing since is the
silently-degrading shape.

## 4. Opening an issue from the board (G2)

The HUD spec names no destinations, and that rule stays. It exists because a
model drafts specs and dataset values are third-party: a link either can
write is a way to send something out, and the owner's click is the request.

What is added is one **typed reference**, where the destination is built by
our code and neither the spec nor the data supplies an address:

```json
{ "type": "table", "dataset": "new_items",
  "columns": ["repo", "title", "age", "summary",
              { "ref": "github", "owner": "owner", "repo": "name", "number": "number" }] }
```

- The spec names three **fields**. It cannot name a host, a path or a query.
- **The spec gate is the control** (`hud/spec.rs`, the typed shape with
  `deny_unknown_fields`): a column becomes an enum of a field name or a
  reference, the reference's `ref` is a closed set holding `github` alone,
  and an unknown `ref` or an unknown key beside the three is refused at
  validation, before any renderer sees it. The checks below are the
  convenience behind it, as `LIVE-DASHBOARD-DESIGN.md` §2.2 splits them.
- The renderer builds `https://github.com/{owner}/{repo}/issues/{number}`
  and nothing else. GitHub redirects that path to the pull request when the
  number is one, so there is one form.
- `owner` and `repo` must each match GitHub's own name grammar and `number`
  must be a positive integer, checked in the renderer and again where the
  dataset is served. The grammar refuses `.` and `..` as a whole name and
  any `%`, so no value can walk the path or smuggle an escape. A value that
  fails renders as its text, no anchor.
- **The pair must be a repository the collector both saw and selected** —
  a row of `inventory` that the selection file includes — checked where the
  dataset is served. The selection file alone is not enough: an owner with
  `include = "all"`, which G6 makes the default, names no repositories, so
  the repo half would be unconstrained. Without this check an issue's text
  could not move the link, but a poisoned row could still point a click at
  a repository of a stranger's choosing.
- The anchor carries `rel="noopener noreferrer"` and a `no-referrer` policy,
  so GitHub is not told which board the click came from.
- **Tailnet boards only.** A board published to the factory renders the
  reference as text until a separate ruling says otherwise; a public page
  with outbound links is a different argument.

`web/test/hud.mjs` scans the renderer source for link sinks today. The typed
reference is the one allowed site, added to that scan by name, so a second
one cannot arrive unnoticed.

The form generalises — a mail thread, a calendar event, a board task are all
"a fixed destination built from typed values" — but each is its own ruling
and its own allowlist; this file adds `github` only.

## 5. The quick summary

An issue body is text a stranger wrote, addressed to the maintainer. It is
exactly the content `mail_triage.rs` exists for, and the summary is made the
same way: `quarantine.rs`'s one-shot, no tools and no history, one item per
call, returning a typed value — a summary of bounded length and a closed
`kind` (bug, question, request, other).

- **Never in the refresh path.** `LIVE-DASHBOARD-DESIGN.md` §9 rules out a
  model in a board's refresh "for any reason". The collector writes the
  summary into `summaries` when it first sees a body, and the loader reads a
  column. The board's refresh stays a reviewed read.
- **Model prose on a board is still model prose.** The same §9 says a
  summary published is reviewed as a publish. On the tailnet board it is the
  owner reading their own tool's output; if this board is ever published,
  the `summary` column does not go with it.
- **The privileged side reads it only as third-party content.** A run
  that drafts or edits a board can see rows: `hud_sources` returns a few
  rows of every table in a registered source and of a query over it, and
  `hud_preview` returns each dataset's first rows. What makes that safe is
  not the absence of a tool but the source's `content = "third-party"`
  (§1): those results are `.from_outside()`, so the interlock arms
  `untrusted` and the run loses chosen egress. If a later step lets a run
  act on an issue (§8), it reads the issue the same way.
- A summary is keyed by a hash of the body, so an unchanged issue is never
  summarised twice, and an edited one is. The pass takes a background seat
  through `permit.rs` and yields like any other; a morning with forty new
  issues is forty short calls behind whatever the owner is doing.
- A body the pass refuses or cannot summarise shows the title alone and says
  so. No summary is not an empty summary.

## 6. The board

Installed by the owner like any board (`~/.mecha/hud/boards/github/`). Three
views, in the order the owner asked for them:

1. **Failing now** — repositories whose default branch is red, longest
   first, with since-when and the workflow's name; beside it a strip of the
   last runs per repository, which is what tells flaky from broken.
2. **New** — issues and outside pull requests (G5) since the owner last
   looked, newest first: repository, title, author, age, the summary, and
   the reference (§4). "Since last looked" is the browser's, not a store's;
   a KPI counts what arrived in the last day and the last week.
3. **Use** — per repository: clones and unique cloners over the kept
   history, release downloads, stars, each with a small trend, sortable.

Filters: owner, and active against dormant. All four panel types exist
today; the only renderer work is §4 and, if the run strip does not come out
of the Vega-Lite subset cleanly, that one chart.

## 7. Build order

Each step is useful alone and is its own pull request.

| Step | What | Gate |
|---|---|---|
| 1 | `mecha github collect`, the selection file read, `inventory`, `ci_runs`, `items`, `counts`, `collect_log`; a systemd timer beside `mecha-triggers.service` | a run against the owner's login fills the tables; a revoked login is a `doctor` finding, not an empty table |
| 2 | The board, installed: views 1 and 2 without summaries or links — `scripts/hud/github/` | the owner opens it and it is right about one known red repository |
| 3 ✓ | The typed reference (§4): spec gate, renderer and serve-side check — *built 2026-11-22, §7.1* | the spec gate refuses an unknown `ref` and an unknown key beside the three; a row naming a repository outside inventory ∩ selection, or `..` as a name, renders no anchor; `hud.mjs` fails on a second link sink |
| 4 | Summaries (§5) — **built** (below) | an issue whose body is an instruction to the reader produces a summary and nothing else; unchanged bodies are not re-summarised |
| 5 ✓ | `traffic`, `releases`, view 3; then the package mapping and registry counts — *built 2026-11-22 but for conda and container counts, §7.2* | totals carry their since-date; a repository without push access shows a dash |
| 6 ✓ | The picker page (G3) — *built 2026-11-22, §7.3* | select-all on an owner writes the file the collector then obeys |

**Step 4, as built** (`github/summary.rs`, `mecha github collect`). The
listing asks for each `body` (a scalar: no change to the batch's node
cost); the body is held in memory, never written, and `items.body_hash` is
its sha256. Its bytes are capped at the door: every `gh` answer is drained
to at most `ANSWER_BYTES_MAX` (32 MiB) and one past it is refused whole
(`GhError::TooLarge`), never parsed cut short; an items listing refused for
its size is asked again without `body`, so its items land unhashed, wait
for a later run (counted deferred), and the loss is logged
`summary_skipped` — which never marks a repository not read in full or
fails the unit. After the collection is recorded, the pass asks about each body
whose hash has no `summaries` row, newest first, at most `PER_RUN` (40) a
run within `BUDGET` (5 min, seat waits included; the unit's start timeout
rose to 900 s to hold both budgets). Each is a `QuarantinedPass` one-shot,
`local_only` and asked not to think, returning `{summary, kind}` and
nothing else (`deny_unknown_fields`); one line, at most 300 characters
(asked for 200), a kind from bug/question/request/other. The schema sent
to a structured-output endpoint bounds the shape and never the length:
llama.cpp compiles `maxLength` into a grammar that clips the answer
mid-word, which the parse would then accept (review of #909). Anything else is a
refusal from a closed set (`declined`, `tool_call`, `not_json`,
`malformed`, `unknown_kind`, `empty`, `too_long`, `no_answer`), stored in
place of a summary; the model's own text is never stored or logged. One
retry, told why in our words, never for a provider refusal or a tool call.
A seat per call (`permit.rs`); the command holds the router for the pass
alone, refusing rather than waiting on a pending switch, and observes the
router after its hold; "switch now" stops it. No local model, no seat, an
outage, a switch or a spent budget logs `summary_skipped`; an item's own
provider failure logs `summary_failed` (by class, never the server's text)
and is asked again. The run's `finished` row carries summarised / refused /
failed / deferred counts, `NULL` when the pass did not run
(`--no-summaries`). `mecha doctor` reports a skipped pass with its streak,
failed calls, and a run whose every answer was refused. The loader joins
the summary of the current hash and says why where there is none.

Decisions taken in the build, open to the owner: **a refusal is final for
the model that gave it** — another model is asked again, so a day on the
wrong model does not cost those items their summaries for good;
**this machine's model only**, and the pass reads the global config alone,
since private repositories' bodies are the owner's; the per-run cap of 40
and the 5-minute budget; summary problems are logged for the doctor and do
not make the unit fail, since the collection they follow was written whole.

It is an optional feature, so step 1 starts at `ARCHITECTURE.md` §Features →
*Adding a feature*. From step 6 the selection file is the picker's to edit
(§7.3), and a hand edit is as good as one.

### 7.1 Step 3: what was built (2026-11-22)

- **The gate** (`hud/spec.rs`): a table column is `TableColumn` — a field
  name, or `GithubRef { ref: RefKind, owner, repo, number }` with
  `deny_unknown_fields`, where `RefKind` holds `Github` alone. A refusal
  states the whole rule, since its reader is a model that will retry. The
  three fields are column names like any other. `cross_check` also requires
  the loader to declare them, with `number` as an `integer` and `owner`/`repo`
  as `string`s, because a string number would draw as text on every row.
- **The served check** (`hud/link.rs`, `github::Watched`, `serve/hud.rs`):
  the route reads which columns are references from the checked spec
  (`Spec::github_refs`), never from the rows. When there are any, the dataset
  goes out with `linkable`, the `[owner, repo, number]` of each row in
  GitHub's grammar whose pair is in `inventory` and in the selection file *as
  it reads now*. The comparison ignores case, as GitHub's names do. An absent
  store, or an absent or unusable file, gives an empty set. The list's digest
  goes into the `ETag`, so dropping a repository from the selection stops its
  links on the next poll.
- **The renderer** (`hud.js` `githubHref`, `IssueLink.svelte`): an anchor
  needs three things. The dashboard's `links` must be on, which only the
  app's installed-board route sets. The grammar must pass again. And
  `linkable` must hold that exact triple. An anchor carries `target="_blank"`,
  `rel="noopener noreferrer"` and `referrerpolicy="no-referrer"`. A reference
  draws as `#<number>` under a `GitHub` header, and as the cell's text when
  it fails any check.
- **Tailnet only, twice over.** The standalone bundle a publish vendors
  mounts the dashboard without `links`. Factory datasets are pushed from the
  store and never pass the serve route, so they carry no `linkable`. A
  proposal is drawn without `links` too.
- **`hud.mjs`** scans the renderer for every link sink: an anchor or area,
  `href` in any spelling, an element spread, `setAttribute('href')`,
  `window.open`, a location write. It requires exactly the anchor and the one
  `href={githubHref(…)}` in `IssueLink.svelte`. It also requires that
  `https://github.com/` appears once, in `hud.js`. Separately, a Svelte
  server render of `Table.svelte` reads the anchors off the markup.
- **The shipped board's** New table has the reference in place of its
  `number` column, and `new_items` selects `i.name` for it. A board
  installed before this keeps its old files until the owner reinstalls it.

### 7.2 Step 5: what was built (2026-11-22)

`github/usage.rs` (traffic, releases, the mapping), `github/registry.rs`
(the counts), and the board's `usage` and `packages` loaders.

- **Traffic** is two REST calls a repository (`traffic/clones`,
  `traffic/views`), upserted per day into `traffic` and kept for good, so
  GitHub's fourteen days accumulate from the first run on. A day the answer
  listed takes the newest number. A day of its window it did not list is a
  zero, because the answer covered it, but is never written over a number an
  earlier answer gave. A day no answer covered has no row. The board's total
  runs from the first day read (`clones_since`) and counts the uncovered days
  as `missing_days`, never as zeros. At most `TRAFFIC_PER_RUN` (150)
  repositories are read a run, the never-read and then the longest-unread
  first, so a selection of four hundred is read every three runs, inside the
  window.
- **Push access** comes from the REST `permissions.push` that the login's
  listing already carries (`inventory.push`, a column added in place). A
  repository it says "no" for is not asked. One GitHub refuses anyway
  (`Must have push access … (HTTP 403)`, `GhError::NoAccess`) logs
  `traffic_no_access`, a refusal and not a failure: it neither marks the
  repository unread nor fails the unit, and the doctor names it. Either way
  `traffic_access.state` is `no_access` and the board shows a dash. Any other
  403 stays a failure.
- **Release downloads** ride the existing GraphQL batch, so they cost no new
  call: the twenty newest releases' assets, fifty each, a row per asset per
  day in `releases`, the day's latest answer standing for it, with a
  `release_reads` row saying how many there were and whether that was all.
  They are GitHub's all-time per-asset counts. Generated source archives are
  not counted, and a repository with no asset shows a dash.
- **Stars** are `counts`, as before; the board adds the change over thirty
  days, a dash until a sample that old exists.
- **The mapping** is read from each repository's own manifests, in one
  GraphQL query per ten repositories (`object(expression: "HEAD:…")`):
  `pyproject.toml` (`[project]` or `[tool.poetry]`), else `setup.cfg`'s
  `[metadata]`, for PyPI; `package.json` unless it is `private`, for npm;
  `recipe/` or `conda.recipe/meta.yaml` (a literal, or `{% set name %}`) for
  conda. `setup.py` is not read, because it is a program. Text is held in
  memory, and only a name that passes the registry's grammar is kept,
  canonicalised. Manifests are read again after a push to the default branch
  or after 30 days, at most 50 repositories a run. A repository with none
  records a scan that found none, and the board says "none found".
- **The owner's corrections** are `~/.mecha/github/packages.toml`
  (`[[package]] repo, registry, name`, or `none = true`;
  `deny_unknown_fields`), which the picker (step 6) will write and the
  collector only reads. A line decides its `(repo, registry)` either way, and
  the detected name stays beside it (`packages.detected_name`) for the
  picker. A file that cannot be used logs `bad_packages` and leaves the
  mapping as the last run that could read it wrote it. A line naming a
  repository in no inventory and no manifest read is not written and logs
  `package_unmatched`, which has its own doctor finding and fails nothing,
  since a repository deselected or renamed later leaves such a line behind.
- **Registry counts** are a pass after the write, like the summaries:
  pypistats.org for PyPI (PyPI's own JSON API no longer reports downloads)
  and npm's downloads API, a month of daily counts per name into
  `registry_daily`, kept for good. It is a fixed fetch with no model and no
  credential: the collector has no tool surface, the hosts are constants,
  the name passes the grammar again before it goes into the path, and
  redirects are not followed. At most 60 names are asked within
  `registry::BUDGET` (60 s), the longest-unchecked first. A 429 or 403 is
  `registry_refused` and stops that registry for the run; no answer, a server
  error or an answer that does not parse is `registry_failed`; a 404 is an
  answer (`not_found`), shown on the board and not logged. Neither fails the
  unit, and `--no-registries` skips the pass.
- **The budget**: `RUN_BUDGET` (8 min), the registries' minute, the
  summaries' 5 min and a minute for the writes make the unit's 900 s, held by
  `the_budgets_fit_inside_the_units_start_timeout`. The traffic and manifest
  calls are inside `RUN_BUDGET`, and their quota is on the `finished` row.
- **The board** has two tables over `usage` (clones and views; downloads and
  stars) and one over `packages`. Each figure has a thirty-day strip drawn in
  its cell as block characters, a dot for a day with no reading, because a
  table cell holds no chart. Rows are ordered by clones. The renderer's
  tables do not sort, so "sortable" (§6) is not met.

Decisions taken in the build, open to the owner:

- **A private repository's detected package name is never sent to a
  registry**, since asking would tell a third party it exists. Only an
  owner-set name is asked about.
- **Unique cloners over the whole history cannot be counted.** GitHub gives
  them per day and per fourteen days, so the board shows each day's figure
  summed (`unique_cloner_days`, named for what it is) beside GitHub's
  fourteen-day figure.
- The per-run caps: 150 traffic repositories, 50 manifest reads, 60 registry
  names.
- The zero-fill of an answered window's unlisted days assumes GitHub's
  window is the fourteen days ending today. That is unmeasured until the
  first real run, and its `traffic` rows are the first thing to check.

**Not built:** conda download counts (names are detected and mapped, and
shown "not counted yet"; anaconda.org's API needs a channel per name),
container pulls, and click-to-sort in the table renderer.

### 7.3 Step 6: what was built (2026-11-22)

`github/picker.rs` (the read and the edit), `serve/github.rs` (three
routes), and Settings → GitHub (`SettingsGithub.svelte`, `github-picker.js`).

- **The read** (`GET /api/github/picker`) lists, per owner, every repository
  in the collector's `inventory`: the login's own account and each
  organisation it saw, and any owner the file names. Each row has fork,
  archived, private, pushed-at, and whether the last collect watched it
  (`inventory.selected`). It also has whether the file **as it reads now**
  selects it (`Selection::selects`, the collector's own rule), what its
  owner's line alone gives it, and its own `[[repo]]` line. The owner's-line
  answer is `Selection::by_owner`, split out of `selects` so the two cannot
  drift. Each registry shows a mapping: `packages.toml`'s name or `none` as
  the file reads now, else the detected name and its manifest, else "none
  found" for a scanned repository, else "not scanned yet". A `[[repo]]` line
  naming a repository the inventory does not hold is listed under its owner
  as not seen yet. The read never calls GitHub. No store, a store that cannot
  be read, and a file that cannot be used are each a state the page says
  (`StoreView`, `FileState`), never an empty list. A selection that cannot be
  read shows every repository's inclusion as unknown, not unselected. The
  last finished collect comes from `collect_log`, and the list's own age from
  `inventory.seen_at`; the page says when the last run could not list. The
  schedule is the shipped timer's `OnCalendar` line, read from
  `scripts/mecha-github-collect.timer` at build time and labelled as
  shipped.
- **The write** (`POST …/selection`, `POST …/packages`) takes a closed set
  of changes (`deny_unknown_fields`):
  - `select_all`: the owner's `include = "all"`, with the owner's `[[repo]]`
    exclusions removed, or select-all would not select all.
  - `select_listed`: `include = "listed"`, touching no `[[repo]]` line. It
    is the undo of `select_all`, so the owner's hand-made inclusions come
    back rather than go (review of #913: unticking "all" used to send
    `select_none` and delete them).
  - `select_none`: `include = "listed"`, with the owner's `[[repo]]` lines
    removed, inclusions too. The page offers it as its own action, and asks
    in place first when it would remove an inclusion.
  - `filters`: the owner's `forks` and `archived` (G4).
  - `repo`: one repository's line, set, or removed with `null`.
  - per `(repo, registry)`: a name, `none`, or back to `detected`.

  The page asks for a repository's own line only where the wanted state
  differs from `by_owner`, so the file holds exceptions, not restatements.
  An owner line the page writes says `forks = false` and `archived = false`
  out loud, as §2 asks.
- **The owner's file stays the owner's.** Each change is set in place with
  `toml_edit`, so comments, order and spelling survive. A table the edit
  removes hands its comments to the next table, or to the end of the file.
  A file the page creates begins with a header saying the picker wrote it
  and that hand edits are kept. A file the collector cannot read, or one
  written as an inline list, is refused (409) and left for a hand fix,
  never replaced.
- **Compare-and-swap.** Each save carries the digest (sha256) of the file
  its page read, or `null` for no file. Under one process-wide lock the
  server:
  1. reads the file, and refuses a different digest (409, nothing written);
  2. applies the changes;
  3. runs **the collector's own reader** (`parse_selection`,
     `parse_packages`) over the exact bytes that would land, and refuses
     what it refuses (422);
  4. puts the file in place by a temporary sibling, synced and renamed
     (`imagelib::write_atomic_mode`), keeping the file's mode and writing a
     new file 0600.

  A save answers the page's whole read again, so what the page draws after
  a click is what the collector will now do. Any refusal re-reads.
- **Gating.** The routes are `Owner::Of(Feature::Github)` behind
  `owner_guard`: the owner's login, and on each save the page's
  `x-mecha-request` header with no cross-site `sec-fetch-site`. Nothing a
  model or a tool can reach writes either file: no tool names them, no
  agent path calls `picker::save_*`, and the collector only reads them.
- **The page** is Settings → GitHub. Per owner it has a checkbox for all
  (`select_all`, or back to `select_listed`), a separate "select none" with
  an in-page confirm, and the owner's forks and archived filters. Per
  repository it has a checkbox and tags: fork, archived, private, named,
  and "added / dropped next collect" where the file now differs from what
  the last collect did. Each repository also shows its mapping, with an
  editor per registry. Forks and archived repositories are hidden by default
  with a toggle (G4), but one that is selected, or that the last collect
  watched, is never hidden, so its "dropped next collect" stays in sight. There is a search
  box and a "pushed in the last year" filter, as §2 asks. The page says that
  changes take effect on the next collect, and when the last one finished.
  A finished collect that never listed (no `gh` login, say: the run stops
  early and still writes its `finished` row) is said as such, on the page
  and on the Settings index, never as an empty account or "0 of 0".
- **Acceptance**: `select_all_on_an_owner_writes_the_file_the_collector_then_obeys`
  drives `save_selection`, reads the written file with `read_selection`, runs
  the collector against the fake `gh`, and checks the watched set and
  `Watched`. `select_all_through_the_endpoint_writes_the_file_the_collector_obeys`
  does the same through the route.

Decisions taken in the build, open to the owner:

- **The page can write the first file only once a collect has listed.**
  With the switch on and no `repos.toml`, the feature is unready rather than
  off: the collector lists the inventory and selects nothing, and the page
  then lists it. With the switch off the routes answer `feature_off`, like
  every other feature's.
- **Owners are listed alphabetically.** The inventory does not record which
  owner is the login's own account, so the page cannot put it first.
- **Each click saves at once**, one save at a time, rather than a pending
  list behind a Save button. The server's answer is then always what the
  page shows, and nothing in the page restates the selection's rules.
- **Select-none keeps the owner's line** as `include = "listed"` rather than
  deleting it, and removes the owner's `[[repo]]` lines either way.
- **The schedule shown is the shipped timer's**, not the installed unit's
  next run. Asking `systemctl` from a settings read is the Background page's
  job, and that page already shows the timer.

## 8. Deliberately not in scope

- **Acting on GitHub** — commenting, labelling, re-running a job, opening a
  pull request. Every one needs a credential in reach of a run and an
  outbound call whose text a model wrote, which is the vault's design and
  the outbox's, not this board's.
- **An MCP server for GitHub.** Nothing here needs a model to choose a call.
- **Mecha holding the token.** `gh` keeps its own login; the collector
  shells out to it and never reads the token. When the vault exists the
  collector becomes its first client, and that is a change to one function.
- **Notices** — "red for three days" as a proactive message. It waits for
  the measure of whether the owner acts on proactive messages, so that the
  first notice sent is one that can be counted.
- **Diagnosing a failure** by reading CI logs. A log is untrusted text and
  the reader would be a run; a later design.
- Webhooks. A daily poll (G10), or any cadence down to a few minutes, is
  within the API's limits for the selected set, and nothing listens for
  inbound HTTP today.

## 9. Open

Nothing. Ruled on 2026-11-18 and moved to §0 (G7–G9): the typed
reference, the quarantined summaries, and `gh` as the credential holder;
on 2026-11-19 (G10), how often: once a day to start.
