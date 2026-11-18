# The GitHub board — design

**2026-11-17.** What is being designed: a board that shows, across the
repositories the owner chooses, what is **failing** (CI on the default
branch), what is **new** (issues, and pull requests from outside
contributors, each with a short summary and a way to open it), and how much
each repository is **used** — fed by a collector that runs with no model
holding a credential, and drawn by the HUD.

**Not built.** It extends [`LIVE-DASHBOARD-DESIGN.md`](LIVE-DASHBOARD-DESIGN.md)
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

**Still open** (§9): how often the collector runs.

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
| 2 | The board, installed: views 1 and 2 without summaries or links | the owner opens it and it is right about one known red repository |
| 3 | The typed reference (§4): spec gate, renderer and serve-side check | the spec gate refuses an unknown `ref` and an unknown key beside the three; a row naming a repository outside inventory ∩ selection, or `..` as a name, renders no anchor; `hud.mjs` fails on a second link sink |
| 4 | Summaries (§5) | an issue whose body is an instruction to the reader produces a summary and nothing else; unchanged bodies are not re-summarised |
| 5 | `traffic`, `releases`, view 3; then the package mapping and registry counts | totals carry their since-date; a repository without push access shows a dash |
| 6 | The picker page (G3) | select-all on an owner writes the file the collector then obeys |

It is an optional feature, so step 1 starts at `ARCHITECTURE.md` §Features →
*Adding a feature*. Until step 6 the selection file is edited by hand.

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
- Webhooks. Polling every ten to fifteen minutes is within the API's limits
  for the selected set, and nothing listens for inbound HTTP today.

## 9. Open

Ruled on 2026-11-18 and moved to §0 (G7–G9): the typed reference, the
quarantined summaries, and `gh` as the credential holder.

1. **How often.** Ten minutes for CI and items, daily for traffic and
   counts, is the proposal; the limit that matters is the API's, measured in
   step 1.
